"""Harbor adapter that runs OpenTopia inside the benchmark task container.

The product server and every tool call stay in Harbor's task environment.  This
is intentionally different from the older host-mirroring adapter: copying a
task workspace to the host makes filesystem and process results non-equivalent
to the official Terminal-Bench environment.
"""

from __future__ import annotations

import base64
import json
import secrets
import shlex
import time
from pathlib import Path
from typing import Any

from harbor.agents.base import BaseAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext


class OpenTopiaContainerAgent(BaseAgent):
    """Execute a pinned Linux OpenTopia server in each Harbor environment."""

    SUPPORTS_RESUME = True

    _RUNTIME_DIR = "/tmp/opentopia-eval"
    _SERVER_PATH = f"{_RUNTIME_DIR}/opentopia-server"
    _SERVER_LOG_PATH = f"{_RUNTIME_DIR}/server.log"
    _SERVER_DB_PATH = f"{_RUNTIME_DIR}/opentopia.db"
    _SERVER_PORT = 18787
    # The private evaluation configuration is shared with the repository's
    # existing audit tooling.  Accept those names when Harbor loads its .env
    # file, but expose only the adapter-specific names to the product server.
    _ENV_ALIASES = {
        "OPENTOPIA_EVAL_API_KEY": "AUDIT_COPILOT_LLM_API_KEY",
        "OPENTOPIA_EVAL_BASE_URL": "AUDIT_COPILOT_LLM_BASE_URL",
        "OPENTOPIA_EVAL_MODEL": "AUDIT_COPILOT_LLM_MODEL",
    }

    def __init__(
        self,
        *args: Any,
        server_binary: str,
        reasoning_effort: str = "high",
        max_output_tokens: int = 8192,
        rollout_limit_tokens: int | None = None,
        run_timeout_sec: int = 1800,
        poll_interval_sec: float = 1.0,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        binary = Path(server_binary).expanduser().resolve()
        if not binary.is_file():
            raise ValueError(f"OpenTopia server binary not found: {binary}")
        if reasoning_effort not in {
            "none",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh",
            "max",
        }:
            raise ValueError(f"unsupported reasoning effort: {reasoning_effort}")
        if max_output_tokens < 1 or run_timeout_sec < 1:
            raise ValueError("OpenTopia evaluation budgets must be positive")
        if rollout_limit_tokens is not None and rollout_limit_tokens < 1:
            raise ValueError("rollout_limit_tokens must be positive when configured")

        self._server_binary = binary
        self._reasoning_effort = reasoning_effort
        self._max_output_tokens = max_output_tokens
        self._rollout_limit_tokens = rollout_limit_tokens
        self._run_timeout_sec = run_timeout_sec
        self._poll_interval_sec = poll_interval_sec
        self._auth_token = secrets.token_urlsafe(36)
        self._thread_id: str | None = None
        self._workdir: str | None = None

    @staticmethod
    def name() -> str:
        return "opentopia-container"

    def version(self) -> str:
        return "1.0.0"

    def _required_env(self, name: str) -> str:
        value = self._get_env(name)
        if not value and name in self._ENV_ALIASES:
            value = self._get_env(self._ENV_ALIASES[name])
        if not value:
            raise RuntimeError(f"{name} is required for OpenTopia evaluation")
        return value

    def _server_env(self) -> dict[str, str]:
        return {
            "OPENTOPIA_API_KEY": self._required_env("OPENTOPIA_EVAL_API_KEY"),
            "OPENTOPIA_OPENAI_BASE_URL": self._required_env(
                "OPENTOPIA_EVAL_BASE_URL"
            ),
            "OPENTOPIA_MODEL": self._required_env("OPENTOPIA_EVAL_MODEL"),
            "OPENTOPIA_API_TOKEN": self._auth_token,
            "OPENTOPIA_DB": self._SERVER_DB_PATH,
            # `full-access` is the broadest mode shared by both fixed product
            # snapshots. Harbor's task container remains the outer boundary;
            # pending destructive actions are approved by this noninteractive
            # benchmark adapter below.
            "OPENTOPIA_PERMISSION": "full-access",
            "OPENTOPIA_SANDBOX_MODE": "danger-full-access",
            "OPENTOPIA_SANDBOX_ENFORCEMENT": "disabled",
            "OPENTOPIA_SANDBOX_NETWORK": "inherit",
        }

    async def _exec(
        self,
        environment: BaseEnvironment,
        command: str,
        *,
        env: dict[str, str] | None = None,
        timeout_sec: int = 60,
        user: str | int | None = None,
    ) -> str:
        result = await environment.exec(
            command,
            cwd=self._workdir,
            env=env,
            timeout_sec=timeout_sec,
            user=user,
        )
        if result.return_code != 0:
            detail = (result.stderr or result.stdout or "no command output").strip()
            raise RuntimeError(
                f"OpenTopia container command failed ({result.return_code}): {detail}"
            )
        return result.stdout or ""

    async def _api(
        self,
        environment: BaseEnvironment,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
        *,
        timeout_sec: int = 60,
    ) -> Any:
        body_data = b"" if body is None else json.dumps(body, separators=(",", ":")).encode()
        encoded_body = base64.b64encode(body_data).decode("ascii")
        # Terminal-Bench images do not consistently include Python or curl.
        # Harbor executes task commands through Bash, so its built-in
        # /dev/tcp support is the smallest common transport for the local,
        # authenticated OpenTopia control plane.
        script = "\n".join(
            [
                "set -euo pipefail",
                f"method={shlex.quote(method)}",
                f"request_path={shlex.quote(path)}",
                f"body_b64={shlex.quote(encoded_body)}",
                "body=$(printf %s \"$body_b64\" | base64 -d)",
                "content_length=$(printf %s \"$body\" | wc -c | tr -d ' ')",
                "exec 3<>/dev/tcp/127.0.0.1/" + str(self._SERVER_PORT),
                "printf '%s %s HTTP/1.1\\r\\nHost: 127.0.0.1\\r\\nAuthorization: Bearer %s\\r\\nContent-Type: application/json\\r\\nContent-Length: %s\\r\\nConnection: close\\r\\n\\r\\n%s' \\",
                "  \"$method\" \"$request_path\" \"$OPENTOPIA_EVAL_INTERNAL_TOKEN\" \"$content_length\" \"$body\" >&3",
                "IFS= read -r status_line <&3",
                "response_length=''",
                "transfer_encoding=''",
                "while IFS= read -r header <&3; do",
                "  header=${header%$'\\r'}",
                "  [[ -z \"$header\" ]] && break",
                "  if [[ \"$header\" == [Cc]ontent-[Ll]ength:* ]]; then",
                "    response_length=${header#*:}; response_length=${response_length//[[:space:]]/}",
                "  fi",
                "  if [[ \"$header\" == [Tt]ransfer-[Ee]ncoding:* ]]; then",
                "    transfer_encoding=${header#*:}; transfer_encoding=${transfer_encoding//[[:space:]]/}",
                "  fi",
                "done",
                "if [[ \"$response_length\" =~ ^[0-9]+$ ]]; then",
                "  dd bs=1 count=\"$response_length\" status=none <&3",
                "elif [[ \"$transfer_encoding\" == *chunked* ]]; then",
                "  while IFS= read -r chunk_header <&3; do",
                "    chunk_header=${chunk_header%$'\\r'}",
                "    chunk_size_hex=${chunk_header%%;*}",
                "    [[ \"$chunk_size_hex\" =~ ^[0-9A-Fa-f]+$ ]] || exit 22",
                "    chunk_size=$((16#$chunk_size_hex))",
                "    if (( chunk_size == 0 )); then",
                "      while IFS= read -r trailer <&3; do",
                "        trailer=${trailer%$'\\r'}",
                "        [[ -z \"$trailer\" ]] && break",
                "      done",
                "      break",
                "    fi",
                "    dd bs=1 count=\"$chunk_size\" status=none <&3",
                "    IFS= read -r chunk_terminator <&3",
                "  done",
                "else",
                "  echo \"OpenTopia response had neither Content-Length nor chunked transfer encoding\" >&2",
                "  exit 22",
                "fi",
                "status_code=${status_line#* }",
                "status_code=${status_code%% *}",
                "[[ \"$status_code\" =~ ^2[0-9][0-9]$ ]]",
            ]
        )
        encoded_script = base64.b64encode(script.encode()).decode("ascii")
        command = f"printf %s {shlex.quote(encoded_script)} | base64 -d | bash"
        output = await self._exec(
            environment,
            command,
            env={"OPENTOPIA_EVAL_INTERNAL_TOKEN": self._auth_token},
            timeout_sec=timeout_sec,
        )
        try:
            return json.loads(output)
        except json.JSONDecodeError as error:
            raise RuntimeError(
                f"OpenTopia API {method} {path} returned non-JSON: {output[:500]!r}"
            ) from error

    async def _wait_for_health(self, environment: BaseEnvironment) -> None:
        deadline = time.monotonic() + 30
        last_error = "no health request completed"
        while time.monotonic() < deadline:
            try:
                health = await self._api(environment, "GET", "/health")
                if health.get("ok") is True and health.get("service") == "opentopia-server":
                    return
            except RuntimeError as error:
                last_error = str(error)
            await self._sleep()
        log = await self._server_log(environment)
        raise RuntimeError(
            f"OpenTopia server failed to become healthy: {last_error}; server log: {log}"
        )

    async def _sleep(self) -> None:
        # Keep the adapter simple and deterministic without adding another
        # scheduling dependency to Harbor's agent runtime.
        import asyncio

        await asyncio.sleep(self._poll_interval_sec)

    async def _server_log(self, environment: BaseEnvironment) -> str:
        result = await environment.exec(
            f"tail -c 4000 {self._SERVER_LOG_PATH} 2>/dev/null || true",
            cwd=self._workdir,
            timeout_sec=30,
        )
        return (result.stdout or result.stderr or "no server log").strip()

    async def _configure_provider(self, environment: BaseEnvironment) -> dict[str, str]:
        settings = await self._api(environment, "GET", "/api/settings")
        providers = settings.get("providers") or []
        active_id = settings.get("activeProviderId")
        provider = next((item for item in providers if item.get("id") == active_id), None)
        if provider is None:
            raise RuntimeError("OpenTopia active provider was missing from /api/settings")

        model = self._required_env("OPENTOPIA_EVAL_MODEL")
        provider["model"] = model
        provider["reasoningEffort"] = self._reasoning_effort
        provider["maxOutputTokens"] = self._max_output_tokens
        provider["rolloutBudget"] = (
            {
                "limitTokens": self._rollout_limit_tokens,
                "samplingTokenWeight": 1.0,
                "prefillTokenWeight": 1.0,
            }
            if self._rollout_limit_tokens is not None
            else None
        )
        updated = await self._api(
            environment,
            "PATCH",
            "/api/settings",
            {"providers": providers, "activeProviderId": active_id},
        )
        configured = next(
            (item for item in updated.get("providers", []) if item.get("id") == active_id),
            None,
        )
        if (
            configured is None
            or configured.get("model") != model
            or configured.get("reasoningEffort") != self._reasoning_effort
            or configured.get("maxOutputTokens") != self._max_output_tokens
            or (
                (configured.get("rolloutBudget") or {}).get("limitTokens")
                != self._rollout_limit_tokens
                if self._rollout_limit_tokens is not None
                else configured.get("rolloutBudget") is not None
            )
        ):
            raise RuntimeError("OpenTopia server did not retain the controlled provider settings")
        # Both snapshots expose this connection test.  It makes the provider
        # protocol selection explicit and, on the newer snapshot, persists the
        # adapter-capability profile required before a conversation can start.
        health = await self._api(
            environment,
            "POST",
            "/api/provider/test",
            {"providerId": active_id},
        )
        if not health.get("reachable") or not health.get("modelAvailable"):
            raise RuntimeError(
                "OpenTopia provider test did not confirm a reachable selected model: "
                f"{health.get('error') or health}"
            )
        # Newer snapshots persist per-model adapter profiles. Refuse to start
        # a billable task if a successful health probe did not retain the
        # profile required for the selected HTTP model; otherwise the server
        # fails the turn immediately and Harbor only observes its timeout.
        refreshed = await self._api(environment, "GET", "/api/settings")
        refreshed_provider = next(
            (
                item
                for item in refreshed.get("providers", [])
                if item.get("id") == active_id
            ),
            None,
        )
        if refreshed_provider is None:
            raise RuntimeError("active provider disappeared after capability negotiation")
        profiles = refreshed_provider.get("adapterProfiles")
        if profiles is not None and not profiles.get(model):
            raise RuntimeError(
                "provider capability test completed without an adapter profile for "
                f"the selected model {model}"
            )
        return {
            "model": model,
            "reasoningEffort": self._reasoning_effort,
            "maxOutputTokens": str(self._max_output_tokens),
            "rolloutLimitTokens": (
                str(self._rollout_limit_tokens)
                if self._rollout_limit_tokens is not None
                else None
            ),
            "providerCapabilityProfileTested": "true",
        }

    async def setup(self, environment: BaseEnvironment) -> None:
        self.logs_dir.mkdir(parents=True, exist_ok=True)
        self._workdir = environment.task_env_config.workdir
        if not self._workdir:
            self._workdir = (await self._exec(environment, "pwd", timeout_sec=30)).strip()
        if not self._workdir:
            raise RuntimeError("Harbor task environment did not provide a working directory")

        await self._exec(
            environment,
            f"mkdir -p {self._RUNTIME_DIR} && chmod 777 {self._RUNTIME_DIR}",
            timeout_sec=30,
        )
        await environment.upload_file(self._server_binary, self._SERVER_PATH)
        # Harbor transfers the binary as root so it may not be owned by the task
        # user.  Change only its mode as root, then start OpenTopia as the task
        # user below; this preserves the benchmark's ordinary execution context.
        await self._exec(
            environment,
            f"chmod 755 {self._SERVER_PATH}",
            timeout_sec=30,
            user="root",
        )
        server_command = (
            f"nohup {self._SERVER_PATH} --host 127.0.0.1 --port {self._SERVER_PORT} "
            f"--db {self._SERVER_DB_PATH} --permission full-access "
            f"> {self._SERVER_LOG_PATH} 2>&1 < /dev/null &"
        )
        await self._exec(
            environment,
            server_command,
            env=self._server_env(),
            timeout_sec=30,
        )
        await self._wait_for_health(environment)
        configured = await self._configure_provider(environment)
        (self.logs_dir / "opentopia-controlled-settings.json").write_text(
            json.dumps(
                {
                    "executionRuntime": "harbor-task-container",
                    "workspace": self._workdir,
                    "permissionMode": "full-access",
                    **configured,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    async def _create_thread(self, environment: BaseEnvironment) -> str:
        if self._thread_id is not None:
            return self._thread_id
        thread = await self._api(
            environment,
            "POST",
            "/api/threads",
            {
                "title": self.session_id or "Harbor OpenTopia evaluation",
                "workspaceRoot": self._workdir,
            },
        )
        thread_id = thread.get("id")
        if not isinstance(thread_id, str) or not thread_id:
            raise RuntimeError("OpenTopia did not return a thread id")
        self._thread_id = thread_id
        return thread_id

    async def _approve_pending_actions(
        self, environment: BaseEnvironment, thread_id: str
    ) -> int:
        approvals = await self._api(
            environment,
            "GET",
            f"/api/threads/{thread_id}/approvals?status=pending",
        )
        approved = 0
        for approval in approvals:
            approval_id = approval.get("approvalId")
            if not isinstance(approval_id, str) or not approval_id:
                raise RuntimeError("OpenTopia returned an approval without approvalId")
            await self._api(
                environment,
                "POST",
                f"/api/threads/{thread_id}/approvals/{approval_id}/decision",
                {"approved": True},
            )
            approved += 1
        return approved

    @staticmethod
    def _usage_from_events(events: list[dict[str, Any]]) -> tuple[dict[str, int], bool]:
        usage = {
            "inputTokens": 0,
            "cachedInputTokens": 0,
            "outputTokens": 0,
            "reasoningTokens": 0,
        }
        cache_reported = False
        for event in events:
            payload = event.get("payload") or {}
            if payload.get("type") != "token_usage":
                continue
            usage["inputTokens"] += int(payload.get("input_tokens") or 0)
            usage["outputTokens"] += int(payload.get("output_tokens") or 0)
            usage["reasoningTokens"] += int(payload.get("reasoning_tokens") or 0)
            if "cached_input_tokens" in payload:
                cache_reported = True
                usage["cachedInputTokens"] += int(payload.get("cached_input_tokens") or 0)
        return usage, cache_reported

    @staticmethod
    def _event_metrics(events: list[dict[str, Any]]) -> dict[str, int]:
        """Return content-free interaction counts for paired comparisons."""
        event_types = [
            (event.get("payload") or {}).get("type")
            for event in events
            if isinstance(event, dict)
        ]
        return {
            "modelRequests": sum(event_type == "model_request" for event_type in event_types),
            "providerResponses": sum(
                event_type == "provider_response_received" for event_type in event_types
            ),
            "toolCallsStarted": sum(
                event_type == "tool_call_started" for event_type in event_types
            ),
            "toolCallsFinished": sum(
                event_type == "tool_call_finished" for event_type in event_types
            ),
        }

    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        run_started_at = time.monotonic()
        thread_id = await self._create_thread(environment)
        message = await self._api(
            environment,
            "POST",
            f"/api/threads/{thread_id}/messages",
            {"content": instruction},
        )
        deadline = time.monotonic() + self._run_timeout_sec
        terminal_statuses = {"succeeded", "failed", "cancelled", "interrupted"}
        terminal: dict[str, Any] | None = None
        last_candidate: dict[str, Any] | None = None
        controlled_timeout = False
        approval_count = 0
        while time.monotonic() < deadline:
            candidate = await self._api(
                environment, "GET", f"/api/threads/{thread_id}/turn"
            )
            if isinstance(candidate, dict):
                last_candidate = candidate
            if candidate and candidate.get("status") in terminal_statuses:
                terminal = candidate
                break
            if candidate and candidate.get("status") == "waiting_user_action":
                terminal = candidate
                break
            if candidate and candidate.get("status") == "waiting_approval":
                approval_count += await self._approve_pending_actions(environment, thread_id)
            await self._sleep()

        if terminal is None:
            controlled_timeout = True
            # The server's cancellation endpoint extracts a JSON body even
            # when no specific Turn id is requested.  Sending an empty
            # object preserves the endpoint contract; an absent body yields
            # HTTP 400 and hides the controlled timeout from Harbor.
            await self._api(
                environment,
                "POST",
                f"/api/threads/{thread_id}/turn/cancel",
                {},
            )
            # Retain all observable events after cancellation.  A timeout is
            # a scored failure, but its token, cache, and tool-use data are
            # still essential to a before/after efficiency comparison.
            for _ in range(10):
                candidate = await self._api(
                    environment, "GET", f"/api/threads/{thread_id}/turn"
                )
                if isinstance(candidate, dict):
                    last_candidate = candidate
                if candidate and candidate.get("status") in terminal_statuses:
                    terminal = candidate
                    break
                await self._sleep()
            if terminal is None:
                terminal = last_candidate or {
                    "status": "timed_out",
                    "error": "turn status unavailable after controlled timeout",
                }

        # The conversation projection retains token usage and interaction
        # event types while redacting model request/response bodies.  This
        # keeps the terminal telemetry transfer tractable for tool-heavy tasks.
        events = await self._api(
            environment,
            "GET",
            f"/api/threads/{thread_id}/events?since=0&view=conversation",
            timeout_sec=300,
        )
        if not isinstance(events, list):
            raise RuntimeError("OpenTopia events response was not a list")
        events_path = self.logs_dir / "opentopia-events.json"
        events_path.write_text(json.dumps(events, indent=2) + "\n", encoding="utf-8")
        usage, cache_reported = self._usage_from_events(events)
        event_metrics = self._event_metrics(events)
        context.n_input_tokens = usage["inputTokens"]
        context.n_output_tokens = usage["outputTokens"]
        if cache_reported:
            context.n_cache_tokens = usage["cachedInputTokens"]
        context.metadata = {
            **(context.metadata or {}),
            "executionRuntime": "harbor-task-container",
            "workspace": self._workdir,
            "permissionMode": "full-access",
            "automaticApprovals": approval_count,
            "threadId": thread_id,
            "messageId": message.get("id"),
            "turnStatus": terminal.get("status"),
            "turnError": terminal.get("error"),
            "reasoningEffort": self._reasoning_effort,
            "maxOutputTokens": self._max_output_tokens,
            "rolloutLimitTokens": self._rollout_limit_tokens,
            "reasoningTokens": usage["reasoningTokens"],
            "cacheTelemetry": "provider_reported" if cache_reported else "unsupported",
            "eventCount": len(events),
            **event_metrics,
            "turnElapsedMs": int((time.monotonic() - run_started_at) * 1000),
            "controlledTimeout": controlled_timeout,
        }
        # Let Terminal-Bench run its official verifier even when the model
        # turn timed out or failed.  Raising here makes Harbor discard the
        # collected AgentContext telemetry, turning a scored model failure
        # into an infrastructure failure with null token/cache metrics.  The
        # terminal turn state and controlledTimeout marker above retain the
        # distinction, while the verifier remains the authority for task
        # success.
