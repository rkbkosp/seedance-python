# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "httpx>=0.27,<1",
#   "socksio",
# ]
# ///

from __future__ import annotations

import argparse
import base64
import json
import mimetypes
import os
import sys
import time
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import httpx

PLATFORMS: dict[str, dict[str, str]] = {
    "byteplus": {
        "base_url": "https://ark.ap-southeast.bytepluses.com/api/v3",
        "default_model": "seedance-1-5-pro-251215",
    },
    "volc": {
        "base_url": "https://ark.cn-beijing.volces.com/api/v3",
        "default_model": "doubao-seedance-1-5-pro-251215",
    },
}

IMAGE_ROLE_FIRST = "first_frame"
IMAGE_ROLE_LAST = "last_frame"
IMAGE_ROLE_REFERENCE = "reference_image"


class SeedanceError(RuntimeError):
    pass


def eprint(*args: Any) -> None:
    print(*args, file=sys.stderr)


def _is_url(value: str) -> bool:
    parsed = urlparse(value)
    return parsed.scheme in {"http", "https", "data"}


def _guess_mime(path: Path) -> str:
    mime, _ = mimetypes.guess_type(path.name)
    if mime:
        return mime
    return "application/octet-stream"


def file_to_data_url(path: str) -> str:
    p = Path(path).expanduser().resolve()
    if not p.is_file():
        raise SeedanceError(f"Image file not found: {p}")
    mime = _guess_mime(p)
    encoded = base64.b64encode(p.read_bytes()).decode("ascii")
    return f"data:{mime};base64,{encoded}"


def normalize_image_input(value: str) -> str:
    if _is_url(value):
        return value
    return file_to_data_url(value)


def build_content(
    prompt: str,
    first_frame: str | None,
    last_frame: str | None,
    references: list[str] | None,
) -> list[dict[str, Any]]:
    items: list[dict[str, Any]] = [{"type": "text", "text": prompt}]

    if first_frame:
        items.append(
            {
                "type": "image_url",
                "image_url": {"url": normalize_image_input(first_frame)},
                "role": IMAGE_ROLE_FIRST,
            }
        )

    if last_frame:
        items.append(
            {
                "type": "image_url",
                "image_url": {"url": normalize_image_input(last_frame)},
                "role": IMAGE_ROLE_LAST,
            }
        )

    if references:
        for ref in references:
            items.append(
                {
                    "type": "image_url",
                    "image_url": {"url": normalize_image_input(ref)},
                    "role": IMAGE_ROLE_REFERENCE,
                }
            )

    return items


class SeedanceClient:
    def __init__(self, base_url: str, api_key: str, timeout: float = 300.0) -> None:
        self.base_url = base_url.rstrip("/")
        self.client = httpx.Client(
            base_url=self.base_url,
            timeout=httpx.Timeout(timeout),
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
        )

    def close(self) -> None:
        self.client.close()

    def __enter__(self) -> "SeedanceClient":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    def _request(self, method: str, path: str, *, json_body: dict[str, Any] | None = None) -> dict[str, Any]:
        response = self.client.request(method, path, json=json_body)
        try:
            response.raise_for_status()
        except httpx.HTTPStatusError as exc:
            detail = response.text.strip()
            raise SeedanceError(f"HTTP {response.status_code}: {detail}") from exc

        try:
            data = response.json()
        except ValueError as exc:
            raise SeedanceError(f"Invalid JSON response: {response.text[:500]}") from exc

        if isinstance(data, dict) and data.get("error"):
            raise SeedanceError(json.dumps(data["error"], ensure_ascii=False))
        return data

    def create_task(self, payload: dict[str, Any]) -> dict[str, Any]:
        return self._request("POST", "/contents/generations/tasks", json_body=payload)

    def get_task(self, task_id: str) -> dict[str, Any]:
        return self._request("GET", f"/contents/generations/tasks/{task_id}")

    def download_file(self, url: str, output: str) -> Path:
        out = Path(output).expanduser().resolve()
        out.parent.mkdir(parents=True, exist_ok=True)
        with self.client.stream("GET", url) as response:
            response.raise_for_status()
            with out.open("wb") as f:
                for chunk in response.iter_bytes():
                    f.write(chunk)
        return out


def resolve_api_key(cli_api_key: str | None) -> str:
    if cli_api_key:
        return cli_api_key
    for env_name in ("ARK_API_KEY", "VOLC_ARK_API_KEY", "BYTEPLUS_ARK_API_KEY"):
        value = os.getenv(env_name)
        if value:
            return value
    raise SeedanceError("Missing API key. Use --api-key or set ARK_API_KEY.")


def create_payload(args: argparse.Namespace) -> dict[str, Any]:
    content = build_content(args.prompt, args.first_frame, args.last_frame, args.reference_image)
    payload: dict[str, Any] = {
        "model": args.model,
        "content": content,
    }

    optional_map = {
        "callback_url": args.callback_url,
        "return_last_frame": args.return_last_frame,
        "execution_expires_after": args.execution_expires_after,
        "generate_audio": args.generate_audio,
        "draft": args.draft,
        "camera_fixed": args.camera_fixed,
        "watermark": args.watermark,
        "seed": args.seed,
        "resolution": args.resolution,
        "ratio": args.ratio,
        "duration": args.duration,
        "frames": args.frames,
    }
    for key, value in optional_map.items():
        if value is not None:
            payload[key] = value
    return payload


def pretty_print(data: dict[str, Any]) -> None:
    print(json.dumps(data, ensure_ascii=False, indent=2))


def task_summary(task: dict[str, Any]) -> str:
    status = task.get("status", "unknown")
    content = task.get("content") or {}
    pieces = [f"status={status}"]
    if content.get("video_url"):
        pieces.append(f"video_url={content['video_url']}")
    if content.get("last_frame_url"):
        pieces.append(f"last_frame_url={content['last_frame_url']}")
    if task.get("error"):
        pieces.append(f"error={task['error']}")
    return " | ".join(pieces)


def _extract_progress(task: dict[str, Any]) -> str | None:
    progress = task.get("progress")
    if isinstance(progress, (int, float)):
        return f"{progress:g}%"
    if isinstance(progress, str) and progress.strip():
        return progress.strip()
    return None


def _render_live_status(task: dict[str, Any], elapsed: float, tick: int) -> str:
    spinner = "|/-\\"
    status = task.get("status", "unknown")
    pieces = [spinner[tick % len(spinner)], f"status={status}", f"elapsed={int(elapsed)}s"]
    progress = _extract_progress(task)
    if progress:
        pieces.append(f"progress={progress}")
    return "  ".join(pieces)


def wait_for_task(client: SeedanceClient, task_id: str, poll_interval: float) -> dict[str, Any]:
    start = time.monotonic()
    tick = 0
    last_status: str | None = None
    interactive = sys.stderr.isatty()

    render_interval = 0.1
    next_poll = 0.0
    task: dict[str, Any] = {}

    while True:
        now = time.monotonic()
        if now >= next_poll:
            task = client.get_task(task_id)
            next_poll = now + poll_interval

        status = task.get("status", "unknown")
        elapsed = now - start

        if interactive:
            line = _render_live_status(task, elapsed, tick)
            print(f"\r{line:<80}", end="", file=sys.stderr, flush=True)
        elif status != last_status:
            eprint(f"status={status} | elapsed={int(elapsed)}s")

        if status in {"succeeded", "failed", "cancelled", "expired"}:
            if interactive:
                print("\r" + " " * 100 + "\r", end="", file=sys.stderr, flush=True)
                eprint(task_summary(task))
            return task

        last_status = status
        tick += 1
        time.sleep(render_interval)


def cmd_generate(args: argparse.Namespace) -> int:
    api_key = resolve_api_key(args.api_key)
    with SeedanceClient(base_url=args.base_url, api_key=api_key, timeout=args.timeout) as client:
        payload = create_payload(args)
        create_result = client.create_task(payload)
        task_id = create_result["id"]
        print(f"task_id={task_id}")

        if not args.wait:
            if args.json:
                pretty_print(create_result)
            return 0

        final_task = wait_for_task(client, task_id, args.poll_interval)
        if args.json:
            pretty_print(final_task)

        if final_task.get("status") != "succeeded":
            return 2

        content = final_task.get("content") or {}
        video_url = content.get("video_url")
        last_frame_url = content.get("last_frame_url")

        if video_url:
            print(f"video_url={video_url}")
        if last_frame_url:
            print(f"last_frame_url={last_frame_url}")

        if args.output and video_url:
            path = client.download_file(video_url, args.output)
            print(f"saved_video={path}")

        if args.last_frame_output and last_frame_url:
            path = client.download_file(last_frame_url, args.last_frame_output)
            print(f"saved_last_frame={path}")

        return 0


def cmd_get(args: argparse.Namespace) -> int:
    api_key = resolve_api_key(args.api_key)
    with SeedanceClient(base_url=args.base_url, api_key=api_key, timeout=args.timeout) as client:
        task = client.get_task(args.task_id)
        if args.json:
            pretty_print(task)
        else:
            print(task_summary(task))
        if args.output and (task.get("content") or {}).get("video_url"):
            path = client.download_file(task["content"]["video_url"], args.output)
            print(f"saved_video={path}")
        if args.last_frame_output and (task.get("content") or {}).get("last_frame_url"):
            path = client.download_file(task["content"]["last_frame_url"], args.last_frame_output)
            print(f"saved_last_frame={path}")
        return 0


def platform_defaults(platform: str) -> dict[str, str]:
    try:
        return PLATFORMS[platform]
    except KeyError as exc:
        raise SeedanceError(f"Unsupported platform: {platform}") from exc


def add_shared_connection_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--platform", choices=sorted(PLATFORMS), default="volc", help="Target platform")
    parser.add_argument("--base-url", help="Override the API base URL")
    parser.add_argument("--api-key", help="Override ARK_API_KEY")
    parser.add_argument("--timeout", type=float, default=300.0, help="HTTP timeout in seconds")


def add_generate_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("prompt", help="Video prompt")
    parser.add_argument("--model", help="Model ID override")
    parser.add_argument("--first-frame", help="First frame: local path, http(s) URL, or data URL")
    parser.add_argument("--last-frame", help="Last frame: local path, http(s) URL, or data URL")
    parser.add_argument(
        "--reference-image",
        action="append",
        default=None,
        help="Reference image(s). Only use with models/scenarios that support reference images.",
    )
    parser.add_argument("--callback-url")
    parser.add_argument("--return-last-frame", action=argparse.BooleanOptionalAction, default=None)
    parser.add_argument("--execution-expires-after", type=int)
    parser.add_argument("--generate-audio", action=argparse.BooleanOptionalAction, default=None)
    parser.add_argument("--draft", action=argparse.BooleanOptionalAction, default=None)
    parser.add_argument("--camera-fixed", action=argparse.BooleanOptionalAction, default=None)
    parser.add_argument("--watermark", action=argparse.BooleanOptionalAction, default=None)
    parser.add_argument("--seed", type=int)
    parser.add_argument("--resolution", choices=["480p", "720p", "1080p"])
    parser.add_argument("--ratio", choices=["16:9", "4:3", "1:1", "3:4", "9:16", "21:9", "adaptive"])
    parser.add_argument("--duration", type=int)
    parser.add_argument("--frames", type=int)
    parser.add_argument("--wait", action="store_true", help="Poll until the task finishes")
    parser.add_argument("--poll-interval", type=float, default=3.0)
    parser.add_argument("--output", help="Download the final video to this path")
    parser.add_argument("--last-frame-output", help="Download the returned last frame to this path")
    parser.add_argument("--json", action="store_true", help="Print full JSON")


def finalize_args(args: argparse.Namespace) -> argparse.Namespace:
    defaults = platform_defaults(args.platform)
    if not args.base_url:
        args.base_url = defaults["base_url"]
    if not getattr(args, "model", None):
        args.model = defaults["default_model"]

    if getattr(args, "frames", None) is not None and getattr(args, "duration", None) is not None:
        raise SeedanceError("Use either --duration or --frames, not both.")

    if getattr(args, "last_frame_output", None) and not getattr(args, "return_last_frame", None):
        eprint("warning: --last-frame-output is set, forcing --return-last-frame")
        args.return_last_frame = True

    return args


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Seedance 1.5 video generator for BytePlus and Volcengine (spinner polling)")
    subparsers = parser.add_subparsers(dest="command", required=True)

    gen = subparsers.add_parser("generate", help="Create a video generation task")
    add_shared_connection_args(gen)
    add_generate_args(gen)
    gen.set_defaults(func=cmd_generate)

    get_cmd = subparsers.add_parser("get", help="Get task status/result")
    add_shared_connection_args(get_cmd)
    get_cmd.add_argument("task_id", help="Task ID")
    get_cmd.add_argument("--json", action="store_true", help="Print full JSON")
    get_cmd.add_argument("--output", help="Download the video to this path")
    get_cmd.add_argument("--last-frame-output", help="Download last frame to this path")
    get_cmd.set_defaults(func=cmd_get)

    return parser


def main() -> int:
    parser = build_parser()
    try:
        args = finalize_args(parser.parse_args())
        return args.func(args)
    except KeyboardInterrupt:
        eprint("interrupted")
        return 130
    except SeedanceError as exc:
        eprint(f"error: {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
