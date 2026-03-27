# Seedance CLI

A command-line interface (CLI) tool for the Seedance 1.5 video generation API from BytePlus and Volcengine. It simplifies creating video generation tasks, polling for status updates, and automatically downloading results.

## Features

- **Multi-Platform Support**: Predefined API base URLs for both BytePlus and Volcengine.
- **Smooth Live Status**: Real-time terminal spinner with high-frequency refresh (0.1s) for a modern, responsive experience.
- **Auto-Download**: Automatically saves generated videos (and the last frame) to local paths upon success.
- **Extensive Options**: Configure API Key, model IDs, resolution, aspect ratio, duration, frames, and more via CLI flags or environment variables.
- **JSON Export**: Integrated `--json` flag for easy pipeline automation.

## Quick Start

### Requirements
- Python 3.11+
- Dependencies: `httpx`

### Installation
You can run it directly using [uv](https://github.com/astral-sh/uv) (no manual environment setup required):

```bash
uv run seedance_cli.py --help
```

Alternatively, install dependencies manually:
```bash
pip install httpx
python seedance_cli.py --help
```

### API Key Setup
Set the key via environment variables:
```bash
export ARK_API_KEY="your-api-key"
```

### Usage Examples

#### 1. Generate Video
```bash
python seedance_cli.py generate "A kitten running in the snow" --wait --output cat_running.mp4
```

#### 2. Image-to-Video (First & Last Frame)
```bash
python seedance_cli.py generate "Kitten jumping" --first-frame first.jpg --last-frame last.jpg --wait --output jump.mp4
```

#### 3. Retrieve Task Result
```bash
python seedance_cli.py get <task_id> --output result.mp4
```

## CLI Parameters

| Flag | Description |
| --- | --- |
| `generate` | Create a new video generation task |
| `get` | Check status or download results for a specific task |
| `--platform` | Target platform (`volc` or `byteplus`), defaults to `volc` |
| `--model` | Override the default model ID |
| `--wait` | Poll and show live progress until finished |
| `--poll-interval` | API polling frequency in seconds, defaults to 3.0 |
| `--output` | Local path to save the generated video |
| `--json` | Print raw API responses in JSON format |
