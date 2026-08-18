import { getLoadedApiToken } from "../../platform";
import {
  shouldRecoverEventSequenceGap,
  type EventSequencePolicy,
} from "../../eventStreamSequence";
import { ApiContractError } from "../sseContracts";
import {
  decodeHttpResponse,
  parseHttpResponseJson,
  type HttpContractKey,
} from "../httpContracts";

export type StreamHandle = { close(): void };

export class ApiResponseError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiResponseError";
  }
}

export class ApiTransport {
  private readonly apiToken: string;

  constructor(
    protected readonly baseUrl: string,
    apiToken?: string,
  ) {
    this.apiToken = apiToken || getLoadedApiToken();
  }

  protected async get<T>(
    contract: HttpContractKey,
    path: string,
    signal?: AbortSignal,
  ): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      headers: this.authHeaders(),
      cache: "no-store",
      signal,
    });
    return parseResponse<T>(response, contract);
  }

  protected async post<T>(
    contract: HttpContractKey,
    path: string,
    body: unknown,
  ): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: this.authHeaders(true),
      body: JSON.stringify(body),
    });
    return parseResponse<T>(response, contract);
  }

  protected async patch<T>(
    contract: HttpContractKey,
    path: string,
    body: unknown,
  ): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "PATCH",
      headers: this.authHeaders(true),
      body: JSON.stringify(body),
    });
    return parseResponse<T>(response, contract);
  }

  protected async put<T>(
    contract: HttpContractKey,
    path: string,
    body: unknown,
  ): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "PUT",
      headers: this.authHeaders(true),
      body: JSON.stringify(body),
    });
    return parseResponse<T>(response, contract);
  }

  protected async delete<T>(
    contract: HttpContractKey,
    path: string,
  ): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "DELETE",
      headers: this.authHeaders(),
    });
    return parseResponse<T>(response, contract);
  }

  protected authHeaders(json = false): HeadersInit {
    return {
      authorization: `Bearer ${this.apiToken}`,
      ...(json ? { "content-type": "application/json" } : {}),
    };
  }

  protected openAuthenticatedSse<T extends { seq: number }>(
    path: string,
    decode: (data: string) => T,
    onData: (event: T) => void,
    sequencePolicy: EventSequencePolicy = "contiguous",
  ): StreamHandle {
    const controller = new AbortController();
    let lastSequence = readSince(path);

    const run = async () => {
      while (!controller.signal.aborted) {
        try {
          const response = await fetch(
            withSince(`${this.baseUrl}${path}`, lastSequence),
            {
              headers: {
                ...this.authHeaders(),
                accept: "text/event-stream",
              },
              cache: "no-store",
              signal: controller.signal,
            },
          );
          if (!response.ok) {
            throw new Error(
              `Event stream failed: ${response.status} ${response.statusText}`,
            );
          }
          if (!response.body)
            throw new Error("Event stream response has no body");

          await consumeSse(
            response.body,
            (data) => {
              const event = decode(data);
              const sequence = event.seq;
              if (
                shouldRecoverEventSequenceGap(
                  lastSequence,
                  sequence,
                  sequencePolicy,
                )
              ) {
                console.warn(
                  `OpenTopia event stream skipped sequences ${lastSequence! + 1}-${sequence - 1}; reconnecting to replay them`,
                );
                return false;
              }
              lastSequence = sequence;
              onData(event);
              return true;
            },
            controller.signal,
          );
        } catch (error) {
          if (controller.signal.aborted) break;
          console.error("OpenTopia event stream disconnected", error);
        }
        if (!controller.signal.aborted)
          await abortableDelay(1_000, controller.signal);
      }
    };

    void run();
    return { close: () => controller.abort() };
  }
}
async function consumeSse(
  body: ReadableStream<Uint8Array>,
  onData: (data: string) => boolean | void,
  signal: AbortSignal,
): Promise<void> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (!signal.aborted) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      buffer = buffer.replace(/\r\n/g, "\n");
      let boundary = buffer.indexOf("\n\n");
      while (boundary >= 0) {
        const frame = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        const data = frame
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart())
          .join("\n");
        if (data && onData(data) === false) {
          await reader.cancel("Reconnecting to recover missing events");
          return;
        }
        boundary = buffer.indexOf("\n\n");
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function readSince(path: string): number | undefined {
  const query = path.split("?", 2)[1];
  const value = query ? new URLSearchParams(query).get("since") : null;
  const parsed = value ? Number(value) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : undefined;
}

function withSince(url: string, since: number | undefined): string {
  if (since === undefined) return url;
  const parsed = new URL(url);
  parsed.searchParams.set("since", String(since));
  return parsed.toString();
}

function abortableDelay(
  milliseconds: number,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve) => {
    const timeout = window.setTimeout(resolve, milliseconds);
    signal.addEventListener(
      "abort",
      () => {
        window.clearTimeout(timeout);
        resolve();
      },
      { once: true },
    );
  });
}

export async function parseResponse<T>(
  response: Response,
  contract: HttpContractKey,
): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    throw new ApiResponseError(
      response.status,
      text || `${response.status} ${response.statusText}`,
    );
  }
  if (response.status === 204) return undefined as T;
  const text = await response.text();
  if (!text) {
    throw new ApiContractError(contract, "successful response body is empty");
  }
  const parsed = parseHttpResponseJson(contract, text);
  return decodeHttpResponse<T>(contract, parsed);
}

export function queryString(
  values: Record<string, string | number | undefined>,
): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(values)) {
    if (value !== undefined && value !== "") params.set(key, String(value));
  }
  const query = params.toString();
  return query ? `?${query}` : "";
}
