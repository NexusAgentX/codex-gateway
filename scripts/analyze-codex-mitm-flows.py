#!/usr/bin/env python3
from __future__ import annotations

import argparse
import collections
import json
import sys
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from mitmproxy import http
from mitmproxy.exceptions import FlowReadException
from mitmproxy.io import FlowReader


SENSITIVE_HEADERS = {
    "authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-stainless-arch",
    "x-stainless-lang",
}

SENSITIVE_JSON_KEYS = {
    "access_token",
    "api_key",
    "auth",
    "id_token",
    "key",
    "password",
    "refresh_token",
    "secret",
    "session",
    "token",
}


def redact_header(name: str, value: str) -> str:
    if name.lower() in SENSITIVE_HEADERS:
        return "<redacted>"
    return value


def redact_json(value: Any) -> Any:
    if isinstance(value, dict):
        redacted: dict[str, Any] = {}
        for key, child in value.items():
            if any(part in key.lower() for part in SENSITIVE_JSON_KEYS):
                redacted[key] = "<redacted>"
            else:
                redacted[key] = redact_json(child)
        return redacted
    if isinstance(value, list):
        return [redact_json(item) for item in value]
    return value


def content_type(flow_message: http.Message | None) -> str:
    if flow_message is None:
        return ""
    return flow_message.headers.get("content-type", "").split(";", 1)[0].strip().lower()


def try_json(raw: bytes | None) -> Any | None:
    if not raw:
        return None
    try:
        return json.loads(raw.decode("utf-8"))
    except Exception:
        return None


def summarize_json_payload(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict):
        return {"json_type": type(payload).__name__}

    summary: dict[str, Any] = {
        "keys": sorted(payload.keys()),
    }
    if "model" in payload:
        summary["model"] = payload.get("model")
    if "stream" in payload:
        summary["stream"] = payload.get("stream")
    if isinstance(payload.get("messages"), list):
        summary["messages_count"] = len(payload["messages"])
        roles = []
        for message in payload["messages"]:
            if isinstance(message, dict) and "role" in message:
                roles.append(message["role"])
        if roles:
            summary["message_roles"] = roles
    if isinstance(payload.get("tools"), list):
        summary["tools_count"] = len(payload["tools"])
    if isinstance(payload.get("input"), list):
        summary["input_count"] = len(payload["input"])
    if "usage" in payload:
        summary["usage"] = redact_json(payload["usage"])
    if "error" in payload:
        summary["error"] = redact_json(payload["error"])
    return summary


def summarize_sse(raw: bytes | None) -> dict[str, Any]:
    if not raw:
        return {"events": 0}

    event_count = 0
    done_count = 0
    json_keys: collections.Counter[str] = collections.Counter()
    object_types: collections.Counter[str] = collections.Counter()

    for line in raw.decode("utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line.startswith("data:"):
            continue
        data = line[5:].strip()
        if not data:
            continue
        event_count += 1
        if data == "[DONE]":
            done_count += 1
            continue
        try:
            parsed = json.loads(data)
        except Exception:
            continue
        if isinstance(parsed, dict):
            for key in parsed.keys():
                json_keys[key] += 1
            if isinstance(parsed.get("type"), str):
                object_types[parsed["type"]] += 1
            elif isinstance(parsed.get("object"), str):
                object_types[parsed["object"]] += 1

    return {
        "events": event_count,
        "done_events": done_count,
        "json_keys": dict(json_keys.most_common()),
        "object_or_type": dict(object_types.most_common()),
    }


def endpoint_key(request: http.Request) -> str:
    parsed = urlsplit(request.path)
    return f"{request.method} {parsed.path}"


def print_json(label: str, data: Any) -> None:
    rendered = json.dumps(data, ensure_ascii=False, sort_keys=True)
    print(f"{label}: {rendered}")


def read_flows(path: Path) -> list[http.HTTPFlow]:
    flows: list[http.HTTPFlow] = []
    with path.open("rb") as handle:
        reader = FlowReader(handle)
        try:
            for flow in reader.stream():
                if isinstance(flow, http.HTTPFlow):
                    flows.append(flow)
        except FlowReadException as exc:
            print(f"warning: stopped at unreadable/incomplete flow: {exc}", file=sys.stderr)
    return flows


def main() -> int:
    parser = argparse.ArgumentParser(description="Summarize Codex MITM flows without printing prompt text or secrets.")
    parser.add_argument("flow_file", nargs="?", default="/flows/codex.mitm")
    parser.add_argument("--limit", type=int, default=80)
    args = parser.parse_args()

    flow_file = Path(args.flow_file)
    if not flow_file.exists():
        print(f"missing flow file: {flow_file}", file=sys.stderr)
        return 1

    flows = read_flows(flow_file)
    host_counts: collections.Counter[str] = collections.Counter()
    endpoint_counts: collections.Counter[str] = collections.Counter()
    status_counts: collections.Counter[str] = collections.Counter()
    model_counts: collections.Counter[str] = collections.Counter()
    stream_counts: collections.Counter[str] = collections.Counter()

    print(f"flows: {len(flows)}")
    print()

    for flow in flows:
        request = flow.request
        response = flow.response
        host_counts[request.pretty_host] += 1
        endpoint_counts[f"{request.pretty_host} {endpoint_key(request)}"] += 1
        if response is not None:
            status_counts[str(response.status_code)] += 1

        request_json = try_json(request.raw_content)
        if isinstance(request_json, dict):
            if "model" in request_json:
                model_counts[str(request_json["model"])] += 1
            if "stream" in request_json:
                stream_counts[str(request_json["stream"])] += 1

    for index, flow in enumerate(flows[: args.limit], start=1):
        request = flow.request
        response = flow.response
        request_json = try_json(request.raw_content)

        print(f"#{index} {request.method} {request.pretty_url}")
        if response is None:
            print("status: <no response>")
        else:
            print(f"status: {response.status_code}")
        print(f"request_content_type: {content_type(request)}")
        print(f"response_content_type: {content_type(response)}")

        request_headers = {
            name: redact_header(name, value)
            for name, value in request.headers.items()
            if name.lower() in {"authorization", "content-type", "accept", "user-agent", "openai-beta"}
        }
        print_json("request_headers", request_headers)

        if request_json is not None:
            print_json("request_json", summarize_json_payload(redact_json(request_json)))

        if response is not None:
            if content_type(response) == "text/event-stream":
                print_json("response_sse", summarize_sse(response.raw_content))
            else:
                response_json = try_json(response.raw_content)
                if response_json is not None:
                    print_json("response_json", summarize_json_payload(redact_json(response_json)))
        print()

    print("summary")
    print_json("hosts", dict(host_counts.most_common()))
    print_json("endpoints", dict(endpoint_counts.most_common()))
    print_json("statuses", dict(status_counts.most_common()))
    print_json("models", dict(model_counts.most_common()))
    print_json("stream_flags", dict(stream_counts.most_common()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
