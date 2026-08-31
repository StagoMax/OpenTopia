from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

from evaluation.integrations.harnessbench.trace import materialize_harnessbench_trace


def _event(event_type: str, payload: dict) -> dict:
    return {"type": event_type, "payload": payload}


class HarnessBenchTraceTests(unittest.TestCase):
    def test_materializes_round_usage_tools_and_cache_diagnostics(self) -> None:
        events = [
            _event("application.thread.created", {}),
            _event(
                "opentopia.model_context_built",
                {
                    "request_id": "r1",
                    "round": 1,
                    "stable_prefix_hash": "stable-a",
                    "dynamic_tail_hash": "tail-a",
                },
            ),
            _event("model.request.started", {"requestId": "r1", "round": 1, "attempt": 1}),
            _event("opentopia.model_delta", {"text": "checking"}),
            _event(
                "model.usage",
                {
                    "requestId": "r1",
                    "round": 1,
                    "inputTokens": 100,
                    "outputTokens": 10,
                    "totalTokens": 110,
                    "cachedInputTokens": 40,
                },
            ),
            _event(
                "tool.call.started",
                {"name": "shell", "call": {"id": "call-1", "name": "shell", "input": {"command": "pwd"}}},
            ),
            _event(
                "tool.call.completed",
                {
                    "name": "shell",
                    "result": {
                        "output": "/work",
                        "metadata": {"providerToolCallId": "call-1", "toolName": "shell"},
                    },
                },
            ),
            _event(
                "opentopia.model_context_built",
                {
                    "request_id": "r2",
                    "round": 2,
                    "stable_prefix_hash": "stable-a",
                    "dynamic_tail_hash": "tail-b",
                },
            ),
            _event("model.request.started", {"requestId": "r2", "round": 2, "attempt": 1}),
            _event("opentopia.model_delta", {"text": "done"}),
            _event(
                "model.usage",
                {
                    "requestId": "r2",
                    "round": 2,
                    "inputTokens": 120,
                    "outputTokens": 8,
                    "totalTokens": 128,
                    "cachedInputTokens": 60,
                },
            ),
        ]
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            events_path = root / "events.jsonl"
            events_path.write_text(
                "\n".join(json.dumps(item) for item in events) + "\n",
                encoding="utf-8",
            )
            proxy_dir = root / "usage-proxy"
            result = materialize_harnessbench_trace(
                events_path=events_path,
                proxy_dir=proxy_dir,
                prompts=["do the task"],
                task_id="001-file",
                session_id="session-1",
                model_id="gpt-5.6-terra",
            )

            self.assertEqual(result["rounds"], 2)
            self.assertEqual(result["usage_rows"], 2)
            usage = [
                json.loads(line)
                for line in (proxy_dir / "requests.jsonl").read_text(encoding="utf-8").splitlines()
            ]
            self.assertEqual(usage[0]["input_tokens"], 60)
            self.assertEqual(usage[0]["cache_read_tokens"], 40)
            second = json.loads((proxy_dir / "responses" / "opentopia-0002.json").read_text(encoding="utf-8"))
            request_messages = json.loads(second["request_body"])["messages"]
            self.assertEqual(request_messages[0], {"role": "user", "content": "do the task"})
            self.assertEqual(request_messages[-1]["content"], "/work")
            self.assertEqual(result["cache_observations"][1]["state"], "reused")
            self.assertFalse(result["cache_observations"][1]["stable_prefix_changed"])
            self.assertTrue(result["cache_observations"][1]["dynamic_tail_changed"])

    def test_container_loopback_urls_are_rewritten_without_touching_other_urls(self) -> None:
        harness_src = Path(r"J:\Project\HarnessBench\src")
        sys.path.insert(0, str(harness_src))
        try:
            from evaluation.integrations.harnessbench.opentopia_adapter import (
                _rewrite_container_loopback_urls,
            )
        finally:
            sys.path.remove(str(harness_src))

        prompt = (
            "open http://127.0.0.1:34123/form and "
            "http://localhost:9000/api; keep https://example.com unchanged"
        )
        self.assertEqual(
            _rewrite_container_loopback_urls(prompt),
            "open http://host.docker.internal:34123/form and "
            "http://host.docker.internal:9000/api; keep https://example.com unchanged",
        )

    def test_extended_windows_workspace_path_maps_to_mounted_root(self) -> None:
        harness_src = Path(r"J:\Project\HarnessBench\src")
        sys.path.insert(0, str(harness_src))
        try:
            from evaluation.integrations.harnessbench.opentopia_adapter import (
                _map_workspace,
            )
        finally:
            sys.path.remove(str(harness_src))

        mapped = _map_workspace(
            Path(r"\\?\J:\opentopia-hb-work\run\before\workspace"),
            Path(r"J:\opentopia-hb-work\run"),
            "/bench-work",
        )
        self.assertEqual(mapped, "/bench-work/before/workspace")


if __name__ == "__main__":
    unittest.main()
