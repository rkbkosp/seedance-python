# Seedance CLI

这是一个用于调用 BytePlus (火山引擎) Seedance 1.5 视频生成 API 的命令行工具。它可以方便地创建视频生成任务、轮询状态并自动下载生成的视频。

## 功能特性

- **多平台支持**: 预设了 BytePlus 和火山引擎的 API 地址。
- **实时状态**: 支持在终端显示流畅的旋转进度 (0.1s 刷新率)，让等待过程更直观。
- **自动下载**: 任务完成后可自动将生成的视频(及最后帧)下载到本地。
- **灵活配置**: 支持通过命令行参数或环境变量配置 API Key、模型 ID、生成参数（如分辨率、比例、时长等）。
- **JSON 输出**: 提供 `--json` 选项，方便与其他工具集成。

## 快速开始

### 运行环境
- Python 3.11+
- 依赖项: `httpx`

### 安装
本项目使用了 inline script 格式，你可以直接使用 [uv](https://github.com/astral-sh/uv) 运行，无需手动安装依赖：

```bash
uv run seedance_cli.py --help
```

或者使用 pip 安装依赖后运行：
```bash
pip install httpx
python seedance_cli.py --help
```

### 配置 API Key
可以通过环境变量设置：
```bash
export ARK_API_KEY="your-api-key"
```

### 使用示例

#### 1. 生成视频
```bash
python seedance_cli.py generate "一只在雪地里奔跑的小猫" --wait --output cat_running.mp4
```

#### 2. 使用首尾帧提示
```bash
python seedance_cli.py generate "小猫跳跃" --first-frame first.jpg --last-frame last.jpg --wait --output jump.mp4
```

#### 3. 获取任务状态
```bash
python seedance_cli.py get <task_id> --output result.mp4
```

## 命令行参数说明

| 参数 | 说明 |
| --- | --- |
| `generate` | 创建视频生成任务 |
| `get` | 获取指定任务的状态或下载进度 |
| `--platform` | 选择平台 (`volc` 或 `byteplus`)，默认为 `volc` |
| `--model` | 覆盖默认使用的模型 ID |
| `--wait` | 阻塞并显示进度，直到任务完成 |
| `--poll-interval` | 轮询 API 的间隔时间 (秒)，默认 3.0 |
| `--output` | 自动下载视频到指定路径 |
| `--json` | 以 JSON 格式打印 API 响应 |
