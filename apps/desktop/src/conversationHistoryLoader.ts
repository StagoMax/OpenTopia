import type { ApiClient } from "./api/client";
import type { AgentEvent, Message } from "./types";

const initialMessagePageSize = 60;
const messageCatchUpPageSize = 200;
const eventPageSize = 250;

export type ConversationMessagePage = {
  messages: Message[];
  hasOlderMessages: boolean;
};

/**
 * Owns bounded history I/O. The session controller owns lifecycle and atomic
 * state publication; this loader owns cursor walking and page completeness.
 */
export class ConversationHistoryLoader {
  private readonly client: ApiClient;
  private readonly threadId: string;

  constructor(client: ApiClient, threadId: string) {
    this.client = client;
    this.threadId = threadId;
  }

  async loadInitialMessages(
    signal: AbortSignal,
  ): Promise<ConversationMessagePage> {
    const page = await this.client.listMessages(this.threadId, signal, {
      limit: initialMessagePageSize + 1,
    });
    return trimInitialMessagePage(page);
  }

  async loadOlderMessages(
    before: Message,
    signal: AbortSignal,
  ): Promise<ConversationMessagePage> {
    const page = await this.client.listMessages(this.threadId, signal, {
      before: messageCursor(before),
      limit: initialMessagePageSize + 1,
    });
    return trimInitialMessagePage(page);
  }

  async loadMessageDelta(
    currentMessages: readonly Message[],
    hasOlderMessages: boolean,
    signal: AbortSignal,
  ): Promise<ConversationMessagePage> {
    const latestMessage = currentMessages.at(-1);
    if (!latestMessage) return this.loadInitialMessages(signal);

    const messages: Message[] = [];
    let after = messageCursor(latestMessage);
    while (!signal.aborted) {
      const page = await this.client.listMessages(this.threadId, signal, {
        after,
        limit: messageCatchUpPageSize,
      });
      messages.push(...page);
      const tail = page.at(-1);
      if (page.length < messageCatchUpPageSize || !tail) break;
      const next = messageCursor(tail);
      if (next.createdAt === after.createdAt && next.id === after.id) break;
      after = next;
    }
    return { messages, hasOlderMessages };
  }

  async loadForwardEvents(
    since: number | undefined,
    signal: AbortSignal,
  ): Promise<AgentEvent[]> {
    if (since === undefined) return this.loadInitialEvents([], signal);
    const events: AgentEvent[] = [];
    let cursor = since;
    while (!signal.aborted) {
      const page = await this.client.listConversationEvents(
        this.threadId,
        cursor,
        signal,
        { limit: eventPageSize },
      );
      events.push(...page);
      const tail = page.at(-1);
      if (page.length < eventPageSize || !tail || tail.seq <= cursor) break;
      cursor = tail.seq;
    }
    return events;
  }

  async loadInitialEvents(
    messages: readonly Message[],
    signal: AbortSignal,
  ): Promise<AgentEvent[]> {
    let events = await this.client.listConversationEvents(
      this.threadId,
      undefined,
      signal,
      { limit: eventPageSize },
    );
    const oldestMessage = messages[0];
    while (
      !signal.aborted &&
      oldestMessage &&
      events.length >= eventPageSize &&
      eventCreatedAfterMessage(events[0], oldestMessage)
    ) {
      const before = events[0]?.seq;
      if (before === undefined) break;
      const page = await this.client.listConversationEvents(
        this.threadId,
        undefined,
        signal,
        { before, limit: eventPageSize },
      );
      if (page.length === 0) break;
      events = [...page, ...events];
      if (page.length < eventPageSize) break;
    }
    return events;
  }

  async loadEventsForOlderMessages(
    messages: readonly Message[],
    beforeEventSeq: number | undefined,
    signal: AbortSignal,
  ): Promise<AgentEvent[]> {
    const oldestMessage = messages[0];
    if (!oldestMessage || beforeEventSeq === undefined) return [];

    let cursor = beforeEventSeq;
    let events: AgentEvent[] = [];
    while (!signal.aborted) {
      const page = await this.client.listConversationEvents(
        this.threadId,
        undefined,
        signal,
        { before: cursor, limit: eventPageSize },
      );
      if (page.length === 0) break;
      events = [...page, ...events];
      cursor = page[0].seq;
      if (
        page.length < eventPageSize ||
        !eventCreatedAfterMessage(page[0], oldestMessage)
      ) {
        break;
      }
    }
    return events;
  }
}

function trimInitialMessagePage(page: Message[]): ConversationMessagePage {
  const hasOlderMessages = page.length > initialMessagePageSize;
  return {
    messages: hasOlderMessages
      ? page.slice(page.length - initialMessagePageSize)
      : page,
    hasOlderMessages,
  };
}

function messageCursor(message: Message): Pick<Message, "createdAt" | "id"> {
  return { createdAt: message.createdAt, id: message.id };
}

function eventCreatedAfterMessage(
  event: AgentEvent | undefined,
  message: Message,
): boolean {
  if (!event) return false;
  return Date.parse(event.createdAt) > Date.parse(message.createdAt);
}
