# Siri-Desktop — 桌面语音助手

一个 Windows 桌面语音助手，带一个**简约、有流畅动效的悬浮球**。喊唤醒词即可用中文下达指令：

- 🎙️ **离线语音唤醒 + 识别**（sherpa-onnx，全本地，不联网、不上传）
- 🔊 调系统音量 / 静音
- 💡 调屏幕亮度
- 🚀 打开程序
- 🎵 网易云音乐**按歌名搜索并播放**
- 🧠 规则听不懂的口语，自动交给国内 LLM（DeepSeek 等）兜底理解

## 技术栈

- **UI**：Tauri v2 透明悬浮球（Rust 后端 + Vite/TS 前端）
- **唤醒/识别/断句**：[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)（KWS + SenseVoice ASR + Silero VAD，动态链接 DLL）
- **音量**：Windows Core Audio `IAudioEndpointVolume`
- **亮度**：`brightness` crate（WMI）
- **网易云**：歌名 → 歌曲 ID（网易云搜索 API）→ `orpheus://song/{id}`
- **LLM 兜底**：任意 OpenAI 兼容接口（默认 DeepSeek）

## 环境要求

- Windows 10/11（自带 WebView2）
- [Rust](https://rustup.rs/)（MSVC 工具链）、[Node.js](https://nodejs.org/)
- 一个麦克风

## 快速开始

```powershell
# 1) 安装前端依赖
npm install

# 2) 下载离线模型（KWS / ASR / VAD，约 250MB，存到 src-tauri/models/）
powershell -ExecutionPolicy Bypass -File scripts/fetch-models.ps1

# 3) 运行
npm run tauri dev
```

启动后桌面右下角出现悬浮球。对着麦克风说唤醒词（默认 **“你好问问”**），
悬浮球进入聆听状态，接着说指令，例如：

| 说什么 | 效果 |
|---|---|
| 音量调到 30 / 声音大一点 / 静音 | 调系统音量 |
| 亮度调到 80 / 屏幕暗一点 | 调屏幕亮度 |
| 打开网易云音乐 / 打开微信 | 启动程序 |
| 播放晴天 / 我想听周杰伦的晴天 | 网易云点歌 |
| 下一首 / 暂停 | 媒体控制 |

> 也可以**直接点击悬浮球**手动进入聆听（跳过唤醒词）。
> 托盘图标右键可「显示/隐藏」「编辑配置」「退出」。

## 配置

编辑根目录的 `config.toml`（或复制为 `config.local.toml` 放私密信息，优先级更高、已 gitignore）。

- `wake_word`：仅用于界面显示；真正生效的唤醒词在 `src-tauri/models/kws/keywords.txt`，
  自带 **你好问问 / 小爱同学 / 你好军哥 / 小米小米** 等。
- `[apps]`：把“应用名 → 路径/命令”填好，"打开 XX" 才能精确命中（找不到时会交给系统按名称启动）。
- `[llm]`：填 `api_key` 后启用 LLM 兜底（默认 DeepSeek）。不填则只用规则匹配。
- `[netease]`：`search_api="direct"` 直接走网易云接口；若被风控，部署
  [NeteaseCloudMusicApi](https://github.com/Binaryify/NeteaseCloudMusicApi) 并设 `search_api="service"`。

### 自定义唤醒词

唤醒词需要拼音 token。安装官方 CLI 生成后追加到 `keywords.txt`：

```powershell
pip install sherpa-onnx
"你好小智" | Out-File raw.txt -Encoding utf8
sherpa-onnx-cli text2token --tokens src-tauri/models/kws/tokens.txt --tokens-type ppinyin raw.txt line.txt
# 把 line.txt 的内容追加到 src-tauri/models/kws/keywords.txt，再把 config.toml 的 wake_word 改成显示名
```

## 打包成便携版

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package.ps1
```

会在 `dist-portable/` 生成：`siri-desktop.exe` + 运行时 DLL + `models/` + `config.toml`。
双击 exe 即可运行（模型从同目录的 `models/` 读取）。

## 已知注意事项

- **网易云自动播放**：`orpheus://song/{id}` 在不同客户端版本上行为可能不同（有时只打开歌曲页而不自动播放）。
  如遇到，请确认已登录网易云客户端；后续可加 UI 自动化兜底。
- 第一次 `cargo build` 会下载 sherpa-onnx 预编译库（动态库 DLL），需要联网。
- 台式机外接显示器若不支持 DDC/CI 可能无法调节亮度（笔记本内屏正常）。

## 项目结构

```
src/                  前端：悬浮球 (index.html / main.ts / styles.css)
src-tauri/src/
  lib.rs              入口：窗口/托盘/插件/接线
  pipeline.rs         语音状态机：KWS → VAD → ASR → 意图 → 技能
  audio.rs            麦克风采集 + 重采样到 16k
  wake.rs  asr.rs     sherpa-onnx 封装（唤醒 / 识别+VAD）
  intent/             意图：rules.rs（规则）+ llm.rs（LLM 兜底）
  skills/             技能：volume / brightness / open_app / media / netease
  config.rs           config.toml 读取
scripts/
  fetch-models.ps1    下载离线模型
  package.ps1         打包便携版
```
