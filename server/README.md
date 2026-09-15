# speaklab-server

SpeakLab 的**可选**后端。用 Rust + Axum 写的薄服务。

## 它解决什么

前端 `index.html` 现在完全跑在浏览器里：语音识别用 Web Speech API，
评分是本地启发式算法，数据存 localStorage。**它不需要后端也能用。**

这个服务是为了三件前端做不了或做不好的事：

| 能力 | 为什么需要服务端 |
|---|---|
| **本地语音识别** | 浏览器的 Web Speech API 在部分环境下不可用；本地跑 whisper 不把录音交给第三方 |
| **练习记录同步** | localStorage 换设备就丢；多端同步必须有服务端 |
| **LLM 密钥保管** | 现在密钥存在浏览器里，谁打开开发者工具都能看到 |

所以前端会**探测后端在不在**（打一次 `/api/v1/meta`），
在就用，不在就退回本地那套。**两边都能独立工作。**

## 快速开始

```bash
cd server
cargo run
# 监听 http://127.0.0.1:8787
```

带本地语音识别（需要额外工具链，见下文）：

```powershell
.\scripts\build.ps1 -Whisper
```


不配任何环境变量就能起来。`SPEAKLAB_DEV=1`（默认）会放开 CORS，
方便前端从 `file://` 或别的端口连过来。

### 实测

在开发机上跑过一遍完整链路（tiny.en 模型，CPU 推理）：

```
输入  16kHz 单声道 WAV，4.85 秒
       "Hello, I would like a flat white to go please."

输出  {"text":"Hello, I would like a flat white to go please.",
       "language":"en","duration_secs":4.852125,"elapsed_ms":6312}
```

**逐字正确**。耗时约 6.3 秒识别 4.85 秒音频，tiny 模型在 CPU 上
差不多就是这个速度；换 `base.en` 会更准也更慢。

产物 3.91 MB，加上三个 DLL 共约 6.5 MB。

### 验证

```bash
curl http://127.0.0.1:8787/api/v1/health
curl http://127.0.0.1:8787/api/v1/meta
```

## 接口

全部在 `/api/v1` 下。版本号写进路径，以后改结构不逼客户端跟着升级。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/health` | 存活与运行时长 |
| GET | `/meta` | 能力清单，前端据此决定按钮显不显示 |
| GET | `/practices` | 练习记录列表 |
| POST | `/practices` | 批量写入记录 |
| GET | `/practices/stats` | 汇总统计 |
| POST | `/asr` | 音频转写（multipart） |

### `GET /meta`

```json
{
  "name": "speaklab-server",
  "version": "0.1.0",
  "api_version": 1,
  "capabilities": {
    "asr_local": false,
    "llm_proxy": false,
    "auth_required": false,
    "asr_languages": ["en", "zh", "ja", "ko"]
  }
}
```

`asr_local` 为 `false` 时前端就不该显示「服务端识别」这个选项，
而不是先调用再处理 501。

### `POST /practices`

```json
{
  "records": [
    {
      "id": "r-2026-09-15-001",
      "kind": "shadow",
      "target": "Could I have a flat white to go, please?",
      "transcript": "could i have a flat white to go please",
      "total": 0.9, "accuracy": 0.9, "fluency": 0.9, "completeness": 0.9,
      "label": "Daily",
      "at": "2026-09-15T08:00:00Z"
    }
  ]
}
```

单次最多 500 条。**部分成功不会整体失败**：

```json
{
  "accepted": 1,
  "rejected": [
    { "id": "bad", "reason": "total 应在 0..=1 之间，实际 1.7" }
  ]
}
```

一条坏数据不该毁掉整批同步——客户端可以只重推被拒的那几条。

### `POST /asr`

`multipart/form-data`，字段：

- `audio`：音频文件，**目前只接受 16 位 PCM WAV**
- `language`（可选）：`en` / `zh` / `ja` / `ko` / `auto`，默认取服务端配置

```bash
curl -X POST http://127.0.0.1:8787/api/v1/asr \
  -F "audio=@sample.wav" -F "language=en"
```

> **注意**：浏览器 `MediaRecorder` 默认输出 webm/opus，不是 WAV。
> 前端要么改成录 WAV，要么等这里补一个解码步骤。
> 现在收到非 WAV 会返回 400 并说明原因。

## 错误格式

统一是：

```json
{ "error": { "code": "bad_request", "message": "..." } }
```

`code` 是稳定的机器可读值，**前端应该匹配 code 而不是 message**——
message 是给人看的，会变。

| code | 状态码 | 含义 |
|---|---|---|
| `bad_request` | 400 | 参数问题 |
| `unauthorized` | 401 | 令牌不对 |
| `not_found` | 404 | 资源不存在 |
| `payload_too_large` | 413 | 超过体积上限 |
| `not_implemented` | 501 | 该能力未配置（如没装 whisper 模型） |
| `upstream_error` | 502 | 上游服务问题 |
| `internal` | 500 | 服务端 bug，细节只进日志 |

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `SPEAKLAB_BIND` | `127.0.0.1:8787` | 监听地址 |
| `SPEAKLAB_DEV` | `1` | 放开 CORS；生产环境设为 `0` |
| `SPEAKLAB_ALLOWED_ORIGINS` | 空 | 生产环境允许的来源，逗号分隔 |
| `SPEAKLAB_TOKEN` | 空 | API 令牌；空则不校验 |
| `SPEAKLAB_MAX_BODY_BYTES` | 8 MiB | 单请求体积上限 |
| `SPEAKLAB_ASR_MODEL` | 空 | whisper 模型路径，空则识别不可用 |
| `SPEAKLAB_ASR_LANG` | `en` | 默认识别语言 |
| `SPEAKLAB_ASR_CONCURRENCY` | CPU 核数一半 | 并发推理上限 |
| `SPEAKLAB_LLM_API_KEY` | 空 | 配了才开 LLM 转发 |
| `SPEAKLAB_LLM_BASE_URL` | `https://api.deepseek.com` | |
| `SPEAKLAB_LLM_MODEL` | `deepseek-chat` | |

> **`SPEAKLAB_TOKEN` 为空时接口不校验。** 默认只监听 `127.0.0.1`，
> 这个组合是安全的。但**如果要监听到 `0.0.0.0`，必须同时设令牌**，
> 否则等于把服务敞给整个局域网。启动日志会说明当前状态。

## 本地语音识别（可选）

默认**不编译** whisper，因为要额外工具链。开启步骤：

### 1. 装工具链

```powershell
winget install Kitware.CMake
winget install LLVM.LLVM
```

`whisper-rs` 通过 `bindgen` 生成 FFI 绑定，需要 `libclang.dll`；
C++ 侧通过 cmake 构建，需要 `cmake` 和 `g++`。
本机用的是 rustup 的 GNU 工具链，还需要把 MinGW 加进 `PATH`：

```powershell
$env:PATH = "C:\msys64\mingw64\bin;C:\Program Files\CMake\bin;$env:PATH"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
```

### 2. 拿模型

模型不进仓库（几百 MB）。下一个小号的先试：

```bash
curl -L -o server/models/ggml-base.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
```

### 3. 编译并运行

```powershell
.\scripts\build.ps1 -Whisper
$env:SPEAKLAB_ASR_MODEL = "models\ggml-tiny.en.bin"
.\target\release\speaklab-server.exe
```

`GET /api/v1/meta` 里 `asr_local` 变成 `true` 就说明接上了。

### 关于那三个 DLL

带 `-Whisper` 编出来的 exe，**必须挨着三个 MinGW 的 DLL 才能启动**：

```
libstdc++-6.dll       whisper 的 C++ 运行库
libgcc_s_seh-1.dll    上面那个的依赖
libwinpthread-1.dll   同上
```

少了就报 `0xC0000135 STATUS_DLL_NOT_FOUND`，**而且没有任何输出**：
双击闪一下就没了，控制台也是空的，很难查。

原因是 `whisper-rs-sys` 的构建脚本里写死了：

```
cargo:rustc-link-lib=dylib=stdc++     注意是 dylib
```

这条来自依赖图，`rustflags` 里的 `-static-libstdc++` 压不过它
（rustflags 先生效，随后被这条覆盖）。试过在 `build.rs` 里补
`static=stdc++`，同样无效。

所以 `scripts/build.ps1` 把「编译」和「拷 DLL」合成一步做完。
嫌麻烦就用不带 `-Whisper` 的构建，那个产物没有外部依赖。

### 模型选择

| 模型 | 大小 | 速度 | 备注 |
|---|---|---|---|
| `tiny.en` | 75 MB | 最快 | 准确率一般，适合先跑通 |
| `base.en` | 142 MB | 快 | **日常够用，推荐起点** |
| `small.en` | 466 MB | 中 | 明显更准 |
| `medium.en` | 1.5 GB | 慢 | CPU 上接近实时极限 |

英文练习选 `.en` 结尾的（只认英文，更准更快）。
多语言去掉 `.en` 后缀。

## 设计取舍

**为什么是薄服务。** 路由层只做参数校验和序列化，业务规则在
`domain.rs`，外部依赖在 `asr.rs`。这样 `domain` 能直接单测，
不用起 HTTP 服务。

**为什么并发要限。** whisper 吃满 CPU，放任并发只会让所有请求一起变慢。
用信号量卡住同时在跑的推理数，超出的排队。

**为什么错误码要稳定。** 前端要按错误类型分支（比如 501 就隐藏按钮）。
匹配 message 字符串会在改文案时静默失效。

**为什么内部错误不返回详情。** anyhow 的错误链里可能有文件路径、
连接串。这些进日志，不进响应体。

**为什么没配令牌也放行。** 加一个默认关闭的门禁会让人误以为服务是安全的。
要么真配上，要么明确不加——启动日志会说清楚。

## 目录

```
server/
├── Cargo.toml
└── src/
    ├── main.rs          启动、路由组装、优雅退出
    ├── config.rs        环境变量 → 配置
    ├── state.rs         共享状态、令牌校验
    ├── error.rs         统一错误与状态码映射
    ├── domain.rs        领域模型与规则（无外部依赖，可单测）
    ├── asr.rs           本地语音识别（feature 门控）
    └── routes/
        ├── mod.rs       路由表
        ├── health.rs    health / meta
        └── practices.rs 练习记录与识别
```

## 测试

```bash
cargo test
```

覆盖了分数校验、统计汇总、WAV 解析与重采样、令牌读取、
错误码映射。WAV 相关的测试只在 `--features whisper` 下跑。
