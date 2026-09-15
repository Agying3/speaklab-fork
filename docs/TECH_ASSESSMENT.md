# SpeakLab 技术评估

> 评估对象：`pengp8029-cmd/speaklab` @ `4ab4dc8`
> 评估日期：本次评估
> 方法：静态审计 + 本地起服务实测（Node v24.18.0，Windows）

---

## 一、结论摘要

**本来想挑刺的，挑完这代码只有刺**

前端 3484 行**真能跑**：评分引擎、录音、语料库、统计、打卡，一样不缺。
最没有的只有两个地方：

1. **`server/` 整个目录（279 行），唯一的功能从没成功跑过一次**
2. **README 吹的 5 件事，4 件是假的** —— 作者"bug已修复"关过 3 个 issue，代码一个字没动

**砍掉 `server/` + 前端接线 + README 说实话 = 5 个 issue 全闭，一个新功能都不用写。**

---

## 二、规模量化

| 区块 | 行数 | 占比 | 状态 |
|---|---|---|---|
| CSS（L9–639） | 631 | 18% | 正常 |
| HTML 结构（L640–1014） | 375 | 11% | 正常 |
| JS（L1015–3545） | 2531 | 71% | 正常，少量死代码 |
| `server/server.js` | 279 | — | **可整体删除** |
| **总计** | **3826** | | |

实质代码行（去空行、去纯注释）：**3375 行**（空行 63，纯注释 109）

### 可砍代码量

| 目标 | 位置 | 行数 |
|---|---|---|
| `server/` 整目录 | `server/*` | 279 |
| `Engine` 对象 | `index.html:2066–2092` | 27 |
| `blobToBase64` | `index.html:2093–2100` | 8 |
| `adaptServerResult` | `index.html:2102–2110` | 9 |
| 设置页引擎 UI（HTML） | `index.html:951–965` | 15 |
| 设置页引擎 JS | `index.html:2902–2922` | 21 |
| whisper 分支（4 处散布） | 2113–2121 / 2292 / 2613 / 3306 | ≈15 |
| **合计** | | **≈374 行** |

**砍裁比例约 10%**，且**全部集中在"从未生效的功能"上**，不触碰任何在用能力。

---

## 三、技术栈

### 前端（零依赖，真实）

| 项 | 值 |
|---|---|
| 语言 | 原生 HTML / CSS / JavaScript (ES2020+) |
| 构建 | **无**。无 webpack/vite/babel，无 npm 依赖 |
| 框架 | **无**。无 React/Vue，纯 DOM 操作 |
| 分发 | 单文件 `index.html`（199 KB） |
| 外部请求 | **无 CDN、无外部字体**（已验证：全文件唯一的 `https://` 是 LLM 配置项和 README 文本） |

**浏览器 API 使用统计：**

| API | 用途 | 处数 |
|---|---|---|
| `SpeechRecognition` | 浏览器内置 ASR（默认引擎） | 2 |
| `speechSynthesis` | TTS 示范朗读 | 11 |
| `mediaDevices` / `MediaRecorder` | 录音 | 3 / 5 |
| `AudioContext` | 电平检测、音效合成 | 4 |
| `localStorage` | 全部数据持久化 | 8 |
| `requestAnimationFrame` | 波形动画 | 3 |
| `FileReader` | 导入导出、（原）音频转 base64 | 2 |

### 后端（`server/`，建议删除）

| 项 | 值 |
|---|---|
| 运行时 | Node.js（ESM） |
| 依赖 | `@huggingface/transformers`、`cmu-pronouncing-dictionary`、`wavefile` |
| 模型 | `Xenova/whisper-tiny.en`（int8，约 41 MB，首次下载） |
| 服务 | 原生 `node:http`，无 Express |
| 端口 | 8091 |

---

## 四、核心缺陷（已实测确认）

### 🔴 致命 1：Whisper 打分链路从未成功运行

**证据链（三层，均为确定性）：**

**(a) 服务端只认 WAV** — `wavefile` 源码白名单：
```js
// node_modules/wavefile/lib/riff-file.js:74
this.supported_containers = ['RIFF', 'RIFX'];
```
WebM 头 `1A45DFA3`、Ogg 头 `OggS`、MP4 头 `ftyp` —— **无一能通过**。实测三种全部抛 `Not a supported format.`

**(b) 前端只产 WebM/Ogg/MP4** — `index.html:1213`：
```js
const types=['audio/webm;codecs=opus','audio/webm','audio/mp4','audio/ogg;codecs=opus'];
```

**(c) 中间无任何转码** — `blobToBase64`（`index.html:2093`）用 `readAsDataURL` 直读原始 blob。全文搜索无 WAV 编码器、无 `OfflineAudioContext` 重采样。

**后果**：每次 `/api/score` 必 500 → `voiceScore`（`index.html:2120`）捕获后静默 toast → 退回浏览器识别。
**用户以为在跑本地模型，实际一次都没用上。**

**为啥没人发现**：设置页那个「测试连接」按钮（`index.html:2912`）只打 `/api/health` —— **那端点压根不碰音频**，永远返回 200 给你个绿灯。点一下看到"后端在线 ✓"，谁能想到下面全烂了。

---

### 🔴 致命 2：路径穿越 + 监听全网卡（组合成 LAN 文件读取）

**缺陷 A — 前缀判断缺分隔符**（`server.js:267`）：
```js
if (!full.startsWith(ROOT) || !existsSync(full)) { ... }
```
纯字符串前缀比较。`ROOT = H:\toos\speaklab` 时，`H:\toos\speaklab-secret\x` **通过检查**。

**缺陷 B — 解码早于 resolve**（`server.js:264`）：
```js
let rel = decodeURIComponent(u.pathname);   // %2f 在此变为真分隔符
const full = path.resolve(ROOT, '.' + rel); // 此时 .. 才生效
```
`new URL()` 解析时 `..%2f` 不构成穿越，`decodeURIComponent` 之后才变成 `../`。

**实测复现：**
```
/..%2fspeaklab-secret%2fsecret.txt   → HTTP 200 + 文件内容
/..%5cspeaklab-secret%5csecret.txt   → HTTP 200 + 文件内容
```
（注：`curl` 默认会规范化路径，必须加 `--path-as-is`；fetch 同理。）

**缺陷 C — 未绑定回环地址**（`server.js:275`）：
```js
server.listen(PORT, () => { ... });
```
未传 host，实测 `LocalAddress = ::`（全部网卡），从 LAN IP `10.186.71.44` 访问返回 **200**。
而代码注释（第 9 行）、启动日志（第 276 行）、README 均声称 `127.0.0.1`。

**A + C 叠加 = 同网段任意主机可读取 `speaklab*` 兄弟目录下的文件。**

> 修复（若保留 `server/`）：`listen(PORT, '127.0.0.1')` + 用 `path.relative(ROOT, full)` 替代前缀比较。各一行。

---

### 🟠 严重 3：录音中点「下一句」导致麦克风永久开启

`#btnNext`（`index.html:2151`）**没有** `phase` 守卫，而同类处理函数全部有（场景 2175、难度 2186、`playDemo` 2236）。

```js
$('#btnNext').addEventListener('click',()=>{ this.idx=(this.idx+1)%this.list().length; this.reset(); this.renderSentence(); });
```

`reset()`（2220）将 `phase` 置为 `'ready'` 却不停录音器。连锁反应：
- 20 秒自动停止守卫（2264）条件 `phase==='recording'` 永假 → **永不触发**
- `stopRecording`（2272）首行 `if(this.phase!=='recording') return;` → 直接返回

**麦克风与 ASR 持续运行，无任何路径可停。** 且 `recTimerId` 未清除，计时器继续走。

---

### 🟠 严重 4：弹窗关闭泄漏麦克风，下次点击用旧转写打分

`closeModal`（`index.html:1043`）仅执行 `innerHTML=''`：
```js
function closeModal(){ $('#modalRoot').innerHTML=''; }
```
不调用 `Recorder.stop()` / `ASR.stop()`。取消、Esc、点遮罩（2638/2639/3263）全部走此路径。

`PhoneticsView.recOn`（3285）仅在"停止"分支（3299）复位，且挂在单例上。关窗后 `recOn` 保持 `true` → 下次点击「跟读」时 `if(!this.recOn)`（3269）为假 → 跳过录音，直接取**上次遗留的 `ASR.final`** 打分并写入历史（3323）。

**用户未开口却获得分数。**

---

### 🟡 中等 5：`'auto'` 是死代码，README 承诺的自动启用不存在

```js
mode(){ return Store.get('engine','auto'); }   // index.html:2067
```
但全部 4 个消费点均判断 `=== 'whisper'`（2113 / 2292 / 2613 / 3306）。全文件搜索 `'auto'` 仅出现在下拉框选项（953）与该默认值本身，**无任何探测或自动切换逻辑**。

README L115–127 称同源打开即自动使用 `/api`，实际必须手动切换。**静默失效。**

---

### 🟡 中等 6：服务端若干正确性缺陷（随 `server/` 一并消失）

| 问题 | 位置 | 说明 |
|---|---|---|
| `clamp` 不拦截 NaN | `server.js:84` | `Math.min/max` 放行 NaN；`JSON.stringify` 转为 `null` → 客户端收到 200 + `score:null` |
| `Number(null)===0` 污染末词 | `server.js:73` | Whisper 末段给 `timestamp:[x,null]`，真值判断在数组上 → `end=0` → 时长为负 |
| 无意义目标句白得 0.3 分 | `server.js:204` | 硬编码 `+ 0.2 * 1` |
| `extras` 重复计数 | `server.js:191`（前端 `1422` 同缺陷） | 相似度 <0.75 的配对未加入 `usedB`，该词同时出现在错误词与多余词中 |
| 4 声道下混错误 | `server.js:52` | 仅取 ch0/ch1 并硬除以 2，实测 6 dB 增益误差 |
| `HF_ENDPOINT` 尾斜杠 | `server.js:27` | 文档写法（无尾斜杠）拼出坏 URL，恰好破坏其存在意义 |
| 错误码语义混乱 | `server.js:272` | body 超限应 413 却返回 500，并原样回显内部错误信息 |

---

## 五、README 吹的 vs 实际

**5 条里 4 条是假的。**

| README 说 | 位置 | 实际 |
|---|---|---|
| `Automated tests — 214 assertions` 绿徽章 | L11 | **测试代码从没提交过。** `.pwtools/` 从第一个 commit 就在 `.gitignore` 里躺着。谁 clone 都跑不了 —— 文件根本不存在。这个徽章是纯装饰 |
| `No CDNs, no external fonts — fully usable offline` | L133 | 前半句**是真的**（前端确实零 CDN，查过全文件）。后半句**假的**：默认 ASR 把你声音发 Google 云端，另外还挂着 5 个 LLM 端点 |
| `Dual ASR engines ... optional bundled server` | L65 | "双引擎"这说法本身就建立在假前提上 —— Whisper 那个引擎**根本转不起来**（见致命 1） |
| `Local Whisper engine (optional) — offline transcription` | L84 | 同上。而且模型还得联网下 41 MB，跟"offline"更是半毛钱关系没有 |
| Quick Start Option 1「双击即用」 | L60 附近 | **实测推翻了 issue #1 的归因** —— `file://` 是安全上下文，麦克风能用。详见下方专节 |

### 被打脸那件事

issue #5 是专门来打这个的，作者没得洗：

- **仓库至今只有 2 个 commit**，从 2026-08-17 起零提交
- **#1 / #2 / #3 都回了「bug已修复」然后关掉** —— 但 `index.html:1211` 还是裸的 `getUserMedia`，README L133 那句 `fully usable offline` 一个字没删
- **issue 关了，代码没动。** #5 现在还是 open 状态

### ⚠️ 实测纠正一：issue #1 的归因是错的

issue #1 断言「`file://` 不被视为安全上下文 → `getUserMedia` 被拒绝」。**这个因果链不成立。**

**规范层面（决定性证据）**：MDN [Secure contexts](https://developer.mozilla.org/en-US/docs/Web/Security/Defenses/Secure_Contexts) 明确列出：

| URL | Secure |
|---|---|
| `file:///path/to/resource.html` | ✅ **Secure**（`file` URL） |

同页 "Potentially trustworthy origins" 一节：scheme 为 `https` / `wss` / **`file`** 的来源即为可信来源。**`file://` 被当作安全上下文是规范要求**，不是某个浏览器的实现细节。

**实测层面**（Edge，Playwright 驱动）：

| 场景 | `isSecureContext` | `navigator.mediaDevices` | `getUserMedia` |
|---|---|---|---|
| `file://` 未授权 | `true` | 存在 | `NotAllowedError`（正常权限拒绝） |
| `file://` 已授权 | `true` | 存在 | **`OK`** |
| `http://127.0.0.1` 未授权 | `true` | 存在 | `NotAllowedError` |
| `http://127.0.0.1` 已授权 | `true` | 存在 | **`OK`** |

**两者表现完全一致。** `file://` 下完整录音链路也跑通了：`blob=true, size=2889, mime="audio/webm;codecs=opus"`。

**真实情况**：`file://` 下麦克风**可用**，只需一次用户授权动作。`NotAllowedError` 是任何未授权页面都会有的正常表现，**不是 `file://` 特有缺陷**。

**该修的其实是**：授权与否、是否弹窗，因浏览器与版本而异，而**应用没有任何提示**。用户看到的是「点了没反应」或「录音按钮消失」，无从判断原因。

> README 的正确改法不是「`file://` 不能用，请起服务器」，而是**说清授权要求，并把本地服务器作为推荐路径**。
>
> 实测仅覆盖 Edge（本机未装 Chrome/Firefox），但结论有规范背书，不依赖单一浏览器。

### ⚠️ 实测纠正二：`Recorder.cancel()` 能正常停麦克风

静态审计曾判定 `Recorder.cancel()`（`index.html:1256`）是死代码 —— 理由是它的 `cleanup()` 不清空 `this.rec`，排队中的 `onstop` 仍会触发。

**实测证伪**：

```
录音中        : {"recState":"rec","hasStream":true,"streamActive":true}
调 cancel() 后: {"recState":"idle","hasStream":false,"streamActive":false}
```

麦克风**正常关闭**。`this.rec` 引用残留属实，但不影响功能。**该结论已用于 `closeModal()` 的修复**。

> 教训：这两条都是「先照抄结论、后实测推翻」。静态分析的结论**未经运行验证不应进文档**。

---

## 六、该夸的也得夸

不能光骂。下面这些是**真优点**，砍的时候别误伤：

- **前端零依赖是真的**：没框架、没构建、没 CDN，单文件直接分发。这点名副其实，也是这项目最值钱的地方
- **`alignWords` 写法没问题**：回溯一定终止（我把 0–4 × 0–4 所有组合都穷举过了）
- **`getAsr` 的并发缓存是对的**：3 个并发首次请求只触发 1 次模型加载，没写成经典竞态
- **`Store` 考虑过隐私模式**：`localStorage` 不可用时降级到内存，这个细节想到了
- **注释写得有水平**：关键算法旁边标了设计意图（比如那个 1.05 代价项是干嘛的），不是废话注释
- **CORS 那个 `*` 别夸大**：没 Cookie、没 `ACAO-Credentials`，不算凭证泄露。真实风险有限，我不跟着起哄

---

## 七、丑话说前面（这份评估的局限）

| 项 | 说明 |
|---|---|
| **实机验证覆盖有限** | 麦克风泄漏与弹窗残留两处**已在真实浏览器复现并验证修复**；但只跑了 Edge（本机未装 Chrome/Firefox），跨浏览器差异未穷举 |
| 模型下载看网络 | 靠 `HF_ENDPOINT=https://hf-mirror.com` 才拉下来的 41 MB。你朋友那边不一定下得动 |
| 作者还不知情 | `server/` 是他唯一的"高级功能"卖点。删之前最好打个招呼 |
| LLM 模块没细看 | `index.html:2001` 那块只确认了它存在，没深入测 |
| `extras` 重复计数 | 前端 `scoreShadow` 里也有同款缺陷（`index.html` 约 1422 行），本次未修 —— 它把"已判错"的词又标成"多说" |

---

## 八、怎么下刀

```
砍（374 行，一点不碰在用的东西）
  ├─ server/ 整目录，删
  ├─ 前端 Engine 接线，6 处，删
  ├─ 设置页那个引擎下拉和测试按钮，删
  └─ README 改说实话（撤徽章、"离线"那句、重写 Quick Start）

顺手修真 bug（这条是给 issue #5 的交代）
  ├─ #btnNext 加 phase 守卫       ← 一行
  └─ closeModal 里停录音器         ← 三行，但有个坑，见 PLAN

不修
  └─ Whisper 音频链路 —— 要么重写前端编码器，要么后端接 WebM。
     工作量跟收益完全不成比例，而且功能一删，问题自己就没了
```

**结果：issue #1–#5 全闭，一个新功能都不用写。**
