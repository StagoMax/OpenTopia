"""Harbor adapter that delegates one official task container to local Codex."""

from __future__ import annotations

import asyncio
import json
import time
from pathlib import Path
from typing import Any

from harbor.agents.base import BaseAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from evaluation.integrations.codex_cli import CodexRunResult, run_container_task


class CodexContainerAgent(BaseAgent):
    """Run a controlled local Codex configuration against a Harbor task."""

    SUPPORTS_RESUME = False

    def __init__(
        self,
        *args: Any,
        run_timeout_sec: int = 1800,
        model: str | None = None,
        reasoning_effort: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        if run_timeout_sec < 1:
            raise ValueError("run_timeout_sec must be positive")
        model = model.strip() if model else None
        reasoning_effort = reasoning_effort.strip() if reasoning_effort else None
        if bool(model) != bool(reasoning_effort):
            raise ValueError("model and reasoning_effort must be supplied together")
        self._run_timeout_sec = run_timeout_sec
        self._model = model
        self._reasoning_effort = reasoning_effort
        self._container_id: str | None = None
        self._workdir: str | None = None
        self._task_user: str | None = None

    @staticmethod
    def name() -> str:
        return "local-codex-container"

    def version(self) -> str:
        return "1.0.0"

    async def setup(self, environment: BaseEnvironment) -> None:
        self.logs_dir.mkdir(parents=True, exist_ok=True)
        self._workdir = environment.task_env_config.workdir
        if not self._workdir:
            self._workdir = await self._required_output(environment, "pwd")
        self._container_id = await self._resolve_container_id(environment)
        uid = await self._required_output(environment, "id -u")
        gid = await self._required_output(environment, "id -g")
        self._task_user = f"{uid}:{gid}"
        (self.logs_dir / "codex-controlled-settings.json").write_text(
            json.dumps(
                {
                    "executionRuntime": "local-codex-cli-to-official-container",
                    "modelSelection": (
                        f"codex-explicit-{self._model}"
                        if self._model
                        else "codex-account-default-no-model-flag"
                    ),
                    "reasoningEffort": self._reasoning_effort or "config-default",
                    "modelSelectionMode": (
                        "explicit-cli-overrides"
                        if self._model
                        else "no-explicit-cli-model-flag"
                    ),
                    "containerId": self._container_id,
                    "workspace": self._workdir,
                    "taskUser": self._task_user,
                    "runTimeoutSeconds": self._run_timeout_sec,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        del environment
        if not self._container_id or not self._workdir or not self._task_user:
            raise RuntimeError("Codex task container was not initialized")
        started = time.monotonic()
        result = await asyncio.to_thread(
            run_container_task,
            instruction=instruction,
            container_id=self._container_id,
            workspace=self._workdir,
            user=self._task_user,
            controller_dir=self.logs_dir / "codex-controller",
            logs_dir=self.logs_dir,
            timeout_sec=self._run_timeout_sec,
            model=self._model,
            reasoning_effort=self._reasoning_effort,
        )
        self._apply_result(context, result, time.monotonic() - started)

    async def _resolve_container_id(self, environment: BaseEnvironment) -> str:
        # DockerEnvironment owns the composed task container.  Its compose
        # helper is the only portable way to address the main service when a
        # task has a custom hostname.  Fall back to the ordinary container
        # hostname for other environment implementations.
        compose = getattr(environment, "_run_docker_compose_command", None)
        if callable(compose):
            result = await compose(["ps", "-q", "main"], check=True, timeout_sec=30)
            container_id = (result.stdout or "").strip()
            if container_id:
                return container_id
        return await self._required_output(environment, "hostname")

    @staticmethod
    async def _required_output(environment: BaseEnvironment, command: str) -> str:
        result = await environment.exec(command, timeout_sec=30)
        if result.return_code != 0:
            detail = (result.stderr or result.stdout or "no command output").strip()
            raise RuntimeError(f"could not inspect Codex task container: {detail}")
        value = (result.stdout or "").strip()
        if not value:
            raise RuntimeError(f"Codex task container returned no output for {command!r}")
        return value

    @staticmethod
    def _apply_result(context: AgentContext, result: CodexRunResult, elapsed: float) -> None:
        context.n_input_tokens = result.usage["inputTokens"]
        context.n_output_tokens = result.usage["outputTokens"]
        if result.telemetry["cacheTelemetry"] != "unsupported":
            context.n_cache_tokens = result.usage["cachedInputTokens"]
        context.metadata = {
            **(context.metadata or {}),
            **result.telemetry,
            "reasoningTokens": result.usage["reasoningTokens"],
            "turnElapsedMs": int(elapsed * 1000),
            "turnStatus": "succeeded" if result.turn_succeeded else "failed",
            "turnError": None if result.turn_succeeded else "local Codex turn did not complete",
        }
