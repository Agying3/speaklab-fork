# SpeakLab local Whisper server (`server/server.js`) — security & correctness audit

**Target:** `H:\toos\speaklab\server\server.js` (279 lines), with `server/package.json` and `index.html` as callers.
**Environment:** Node v24.18.0, Windows, deps installed. All claims below were verified by running code on this machine (probe scripts were temporary and have been deleted). No `npm install`, no long-running server started.

**Headline:** three findings materially change the threat model.
1. The static handler is **fully bypassable** on Windows (`/..%2f<name>%2f<file>` reads sibling directories). Proven end-to-end: HTTP 200 with a real sibling file's contents.
2. The server **does not bind to 127.0.0.1** despite its log line and docs — `listen(PORT)` binds `::` and was **reachable over the LAN** in testing. So (1) is not merely local.
3. The Whisper engine is **functionally dead**: the frontend only ever sends WebM/Opus or MP4, and the server only parses WAV. Every `/api/score` call 500s and silently falls back to the browser engine.

---

## 1. CRITICAL — Path traversal: sibling-directory prefix escape in the static handler

**`server/server.js:266-267`**
```js
const full = path.resolve(ROOT, '.' + rel);
if (!full.startsWith(ROOT) || !existsSync(full)) { res.writeHead(404); res.end('not found'); return; }
```

**What's wrong.** Two independent defects compound:

**(a) `startsWith(ROOT)` is a *prefix* test on a string, not a path-containment test.** `ROOT` is `H:\toos\speaklab` (line 21). `H:\toos\speaklab-secret\f` literally starts with `H:\toos\speaklab`, so it passes. The guard never appends `path.sep`, so every sibling directory whose name begins with `speaklab` is inside the "sandbox".

**(b) `decodeURIComponent` (line 264) runs *before* `path.resolve`, so encoded separators are decoded into real path separators.** `%2f` → `/` and `%5c` → `\` both become traversal-capable *after* the URL parser has already decided there is no `..` in the path. This is the practical exploit vector, because a raw `/../` is normalized away by clients and by `http`.

Both branches verified with `node` on this machine:

```
ROOT = "H:\toos\speaklab"
"/../speaklab-secret/x.txt"          -> "H:\toos\speaklab-secret\x.txt"   startsWith(ROOT): true
"/..%2fspeaklab-secret%2fpwned.txt"  -> "H:\toos\speaklab-secret\pwned.txt" startsWith(ROOT): true
"/%2e%2e/speaklab-secret/x.txt"      -> "H:\toos\speaklab-secret\x.txt"   startsWith(ROOT): true
"/..%5cspeaklab-secret%5cx.txt"      -> "H:\toos\speaklab-secret\x.txt"   startsWith(ROOT): true
```

**Impact — proven, not theoretical.** I stood up the exact handler logic, created `H:\toos\speaklab-secret\secret.txt`, and sent **raw socket** requests (no client-side `..` normalization):

```
"/../speaklab-secret/secret.txt"      -> HTTP/1.1 404 Not Found   (normalized/blocked)
"/..%2fspeaklab-secret%2fsecret.txt"  -> HTTP/1.1 200 OK  "TOP-SECRET-SIBLING-DATA"
"/..%5cspeaklab-secret%5csecret.txt"  -> HTTP/1.1 200 OK  "TOP-SECRET-SIBLING-DATA"
```

`/..%2f..%2f..%2fWindows%2fwin.ini` did *not* work (depth exceeds the prefix, so `startsWith` correctly rejects). **The ability to escape is therefore limited to directories that share the `speaklab` name prefix** — but on this very machine `H:\toos` contains `speaklab`, and real users routinely have `speaklab-backup`, `speaklab.bak`, `app-speaklab-data` next to the project. Combined with finding 2 (LAN-exposed), this is a genuine remote file-read primitive, not a local-only curiosity.

**How to confirm.**
```powershell
mkdir H:\toos\speaklab-secret; 'PWNED' > H:\toos\speaklab-secret\secret.txt
curl.exe --path-as-is "http://127.0.0.1:8091/..%2fspeaklab-secret%2fsecret.txt"
# --path-as-is is required; most clients normalize %2f/%2e%2e away
```
**Fix.** Resolve, then compare with a separator-terminated root and reject anything outside:
```js
const full = path.resolve(ROOT, '.' + rel);
const rootWithSep = ROOT.endsWith(path.sep) ? ROOT : ROOT + path.sep;
if (!full.startsWith(rootWithSep)) { /* 403 */ }
```
Better still, drop URI decoding of the path entirely and reject any `rel` containing a null byte, or match `rel` against a whitelist of extensions. Note `path.relative(ROOT, full)` starting with `..` is the idiomatic check.

---

## 2. HIGH — Server binds to all interfaces while claiming 127.0.0.1

**`server/server.js:275-276`**
```js
server.listen(PORT, () => {
  console.log(`[speaklab] Whisper engine on http://127.0.0.1:${PORT}`);
```
and `server.js:9` — `运行：npm install && npm start   → http://127.0.0.1:8091`.

**What's wrong.** `listen(PORT)` passes no host, so Node binds the unspecified address. Measured on this machine:

```
listen(PORT) no host -> address: {"address":"::","family":"IPv6","port":64066}
LAN ip: 192.168.1.9
*** REACHABLE FROM LAN at 192.168.1.9:64067 status 200
```

**Impact.** The log line and README assert local-only, but the port is open on every interface, including IPv4-mapped connections. Any device on the same Wi-Fi/LAN — a shared office network, a café, an untrusted guest VLAN — can reach the static handler and both `/api` routes. This is what upgrades finding 1 and finding 4 from "local only" to "network exposed", and `Access-Control-Allow-Origin: *` (finding 4) then applies to those remote callers too.

**How to confirm.** Above; `netstat -ano | findstr :8091` shows `0.0.0.0:8091`/`[::]:8091` rather than `127.0.0.1:8091`.
**Fix.** `server.listen(PORT, '127.0.0.1', ...)`. (A Host-header check would also help against DNS rebinding but binding correctly is the real fix.)

---

## 3. HIGH — The Whisper engine can never work: WAV-only parser vs WebM/Opus-only client

**`server/server.js:47-49`**
```js
function decodeAudio(buf) {
  const wav = new WaveFile();
  wav.fromBuffer(buf);
```
**`index.html:1213-1215`**
```js
const types=['audio/webm;codecs=opus','audio/webm','audio/mp4','audio/ogg;codecs=opus'];
this.mime = (types.find(t=>{ try{ return MediaRecorder.isTypeSupported(t); }catch(e){ return false; } })) || '';
this.rec = this.mime ? new MediaRecorder(this.stream,{mimeType:this.mime}) : new MediaRecorder(this.stream);
```
**`index.html:1249`** — `new Blob(this.chunks,{type:this.mime||'audio/webm'})`, base64'd verbatim at `index.html:2077` and POSTed as `audio`.

**What's wrong.** `MediaRecorder` never produces WAV. `wavefile.WaveFile` only parses RIFF/WAV. I fed it structurally valid WebM/EBML, Ogg, and MP4 headers:

```
webm/opus (MediaRecorder default) -> THROWS: Error | Not a supported format.
ogg                               -> THROWS: Error | Not a supported format.
mp4/m4a                           -> THROWS: Error | Not a supported format.
```

I grepped `index.html` for any WAV encoder, `OfflineAudioContext`, or PCM re-encoding — **there is none**. The only `decodeAudioData` use (`index.html:1266`) is for local waveform drawing and its output is never uploaded. `server.js` contains no ffmpeg/transcode step.

**Impact.** `/api/transcribe` and `/api/score` throw `Not a supported format.` on every real request. Because `transcribeAudio` is `async`, the synchronous throw becomes a rejected promise, caught at `server.js:270` → HTTP 500. The frontend then throws at `index.html:2085`, `voiceScore` catches it at `index.html:2119` and toasts *"本地 Whisper 不可用…已回退浏览器识别"*. So the headline feature — local Whisper ASR + phoneme scoring — never runs; it silently degrades to the browser engine on every single attempt. This is the most user-visible bug in the file and would be the first thing a user reports.

**How to confirm.** Record audio from the page with the engine set to `whisper`, watch the 500 in the server log, or:
```powershell
curl.exe -X POST --data-binary "@clip.webm" http://127.0.0.1:8091/api/transcribe
# -> {"error":"Not a supported format."}
```
**Fix.** Either decode client-side to WAV before upload (re-encode via `OfflineAudioContext` and write a RIFF header), or add server-side decoding (`ffmpeg`, `@ffmpeg-installer`, or a pure-JS demuxer for the specific container). Note `transcribeAudio` should also validate the buffer and return a 4xx, not a 500.

---

## 4. MEDIUM — `Access-Control-Allow-Origin: *` on all routes enables localhost CSRF / DNS-rebinding reads

**`server/server.js:232-237`** (and the OPTIONS branch at **`server.js:242`**)
```js
res.writeHead(code, {
  'Content-Type': 'application/json; charset=utf-8',
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Headers': 'Content-Type',
  'Access-Control-Allow-Methods': 'GET,POST,OPTIONS'
});
```

**What's wrong — and what is *not*.** The `*` wildcard is honest here: it does not by itself transmit credentials, because the server sets no cookies and issues no auth, and no `Access-Control-Allow-Credentials` is present. So "any site can read a logged-in user's data" is **not** the risk. The real risks are narrower but concrete:

- **Read access to an unauthenticated local service.** Any page the user visits can `fetch('http://127.0.0.1:8091/api/health')` and read the response cross-origin. That leaks the model name and load state — low value alone, but it is a working **port/service scanner** that distinguishes "SpeakLab running" from "not running", usable for fingerprinting the user's machine.
- **The static handler is CORS-exposed too.** The 404/200 paths at `server.js:267` use bare `res.writeHead` with **no** CORS header, so cross-origin *reads* of static files are blocked by the browser. However, combined with finding 1, an attacker who can get the user to load a page while the server runs cannot read `file://`-style escapes directly — this limits finding 1 to non-browser clients (LAN attackers per finding 2) rather than web pages. **Honest assessment: findings 1 + 2 are the exploitable pair; CORS is not what makes them exploitable.** Reporting this as a critical "any website can read your files" would be overstated.
- **DNS rebinding is the genuine browser vector.** With `Host` unchecked and `*` set, an attacker-controlled domain that rebinds to `127.0.0.1` becomes same-origin with the server, so `*` no longer matters and the static handler's (uncertain) traversal could be read. This is real but requires the rebinding setup, and the payload is confined to the `speaklab*` prefix — modest.
- **Resource abuse.** `*` lets any page drive `/api/score`, which triggers a ~40MB model download and heavy CPU inference. A page that loops this is a plausible client-side DoS / disk-fill on the developer's machine.

**How to confirm.** From a page on any origin in the browser console:
```js
await (await fetch('http://127.0.0.1:8091/api/health')).json()  // succeeds and is readable
```
**Fix.** Drop `*` and reflect only the same-origin case, or require a custom header (`X-SpeakLab`) that forces a preflight and reject requests whose `Origin`/`Host` is not `127.0.0.1`/`localhost`. Combined with the finding-2 bind fix, the risk becomes negligible.

---

## 5. MEDIUM — `Number(null) === 0` makes Whisper's final-word timestamp poison the score

**`server/server.js:73-74`**
```js
start: c.timestamp ? Number(c.timestamp[0]) : null,
end: c.timestamp ? Number(c.timestamp[1]) : null
```
**`server/server.js:77-79`**
```js
const duration = words.length && words[words.length - 1].end != null
  ? words[words.length - 1].end
  : (samples.length / sr);
```
**`server/server.js:170`**
```js
const dur = (wInfo.end != null && wInfo.start != null) ? (wInfo.end - wInfo.start) : 0.4;
```

**What's wrong.** The truthiness test is on the *array* `c.timestamp`, not on the elements. Whisper routinely emits `timestamp: [x, null]` for the final chunk. `Number(null)` is `0`, not `null`, so `end` becomes a real `0`. Measured:

```
{name:'end null (real whisper behavior)', timestamp:[0.1,null]}
  -> {"word":"hello","start":0.1,"end":0}
  -> (wInfo.end != null && wInfo.start != null) ? ... : 0.4   => dur used = -0.1
```

Two distinct consequences:

- **Negative word duration.** `0 - 0.1 = -0.1`. `durScore` collapses to 0, and since `score = 0.7*phoneScore + 0.3*durScore`, a perfectly pronounced final word loses 30% of its score: `0.7*1 + 0.3*0 = 0.70` (down from `0.875`). Status drops from `good` to `mid`. **The last word of every sentence is systematically penalized.**
- **`duration` becomes 0 for the whole utterance.** Line 77's `end != null` check passes (0 is not null), so `duration = 0`. Then `wpm` (line 195) is `0` and `silRatio` (line 202) is `0`, so `rateScore` and `silScore` are garbage — `fluency` is computed from a nonexistent timeline.

**How to confirm.** `node -e "console.log(Number(null), 0.1 - Number(null))"` → `0 -0.1`. Or post a WAV and inspect `words[].score` for the final token.
**Fix.** Test the elements, not the array: `start: Array.isArray(c.timestamp) && c.timestamp[0] != null ? Number(c.timestamp[0]) : null` (same for `end`), and guard `dur` against non-positive values.

---

## 6. MEDIUM — NaN propagates into the JSON response as `null`, silently corrupting scores

**`server/server.js:84`**
```js
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
```
**`server/server.js:170-174`**
```js
const dur = (wInfo.end != null && wInfo.start != null) ? (wInfo.end - wInfo.start) : 0.4;
const expected = 0.18 + refPhon.length * 0.09;
const durScore = clamp(1 - Math.abs(dur - expected) / expected * 1.6, 0, 1);
```

**What's wrong.** This `clamp` is a `min`/`max` sandwich, and `Math.min`/`Math.max` **pass NaN through** rather than clamping it:

```
clamp(NaN,0,1) = NaN
```

Every arithmetic site that can produce NaN is then unguarded. Confirmed NaN sources:
- Any missing/undefined timestamp reaches the arithmetic as `undefined - undefined` → `NaN` (line 170) — the `!= null` test only catches `null`, not `undefined`.
- `samples.length / sr` (line 79) is `Infinity` when `sr === 0`, and `NaN` when `sr` is NaN. `wav.fmt.sampleRate` is read unvalidated at line 50.
- Line 199, `t.words[i].start - t.words[i-1].end`, silently yields `0` for `null - null` (JS coercion) but `NaN` for `undefined - undefined`, so gap detection is inconsistent.

The nastiest part is the serialization boundary: **`JSON.stringify` turns NaN and Infinity into `null`** (`JSON.stringify({s:NaN})` → `{"s":null}`), so the frontend receives `score: null, accuracy: null, total: null` with HTTP 200. `index.html:2089` only checks `j.total == null` → the client throws `'响应格式异常'`, but `adaptServerResult` (`index.html:2102-2109`) copies `accuracy`/`fluency`/`completeness` through unchecked, so a *partially* NaN result (e.g. only `words[i].score` is NaN) renders as a null score with no error. Also note `NaN >= 0.82` and `NaN >= 0.6` are both `false`, so such a word is labelled `status:'bad'` — a correct pronunciation reported as a failure.

**How to confirm.** `node -e "const c=(v,a,b)=>Math.min(b,Math.max(a,v)); console.log(c(NaN,0,1), JSON.stringify({s:c(NaN,0,1)}))"` → `NaN {"s":null}`.
**Fix.** `const clamp = (v,a,b) => Number.isFinite(v) ? Math.min(b, Math.max(a, v)) : a;` (or `0`), and validate `sr > 0` after `fromBuffer`.

---

## 7. MEDIUM — Unparseable target scores a free 30%; extra/unmatched speech is never penalized

**`server/server.js:192-193`**
```js
const completeness = T.length ? matched / T.length : 0;
const accuracy = T.length ? accSum / T.length : 0;
```
**`server/server.js:204`**
```js
const fluency = clamp(0.5 * rateScore + 0.3 * silScore + 0.2 * 1, 0, 1);
```

**What's wrong.** When `tokenize(target)` yields no words (`T.length === 0`), `accuracy` and `completeness` are 0 but `fluency` still contains a hard-coded `+ 0.2 * 1` constant and its other terms default to 1 when duration is 0. Verified:

```
target=""      T=[] -> accuracy=0 fluency=1 completeness=0 TOTAL=0.3
target="   "   T=[] -> accuracy=0 fluency=1 completeness=0 TOTAL=0.3
target="!!!"   T=[] -> accuracy=0 fluency=1 completeness=0 TOTAL=0.3
target="😂😂"  T=[] -> accuracy=0 fluency=1 completeness=0 TOTAL=0.3
```

So a target of `"12345"`, `"!!!"`, or emoji — all of which pass the `!target` check at line 257 because the raw string is truthy — grants **30% for saying nothing at all**. The `0.2 * 1` freebie means no response can ever score below `0.3*0.2 = 0.06`, and `fluency` reports a perfect 1.0 for silence.

Separately, `extras` is computed and returned (line 190-191) but **never enters any score**. Verified: target `hello world` against six transcribed words scores `acc=0.831 completeness=1 TOTAL=0.916` — the four hallucinated/extra words cost nothing.

**How to confirm.** `POST /api/score` with `{"audio":"<valid wav>","target":"!!!"}` → `total: 0.3`.
**Fix.** Return 400 when `tokenize(target).length === 0`; remove the `0.2 * 1` constant; fold `extras.length` into the penalty.

---

## 8. LOW/MEDIUM — `readBody` limit is on the raw body, decoded after; malformed input yields 500s and leaks messages

**`server/server.js:218-229`**
```js
function readBody(req, limit = 20 * 1024 * 1024) {
  return new Promise((resolve, reject) => {
    const chunks = []; let size = 0;
    req.on('data', c => {
      size += c.length;
      if (size > limit) { reject(new Error('body too large')); req.destroy(); }
      else chunks.push(c);
    });
```
**`server/server.js:256-258`**
```js
const { audio, target, level } = JSON.parse(body.toString('utf8'));
if (!audio || !target) { json(res, 400, { error: 'audio and target required' }); return; }
const buf = Buffer.from(audio, 'base64');
```

Several concrete issues:

- **The limit is on the raw body, so it is not a memory cap.** A 20MB JSON body becomes a ~20MB `Buffer`, then a **~40MB UTF-16 string** via `body.toString('utf8')`, then a ~15MB decoded audio buffer, then `getSamples(false, Float32Array)` expands it again (measured: 15MB of 16-bit stereo PCM → 30MB of Float32). Peak is roughly 100MB+ per concurrent request, and `readBody` is not serialized. Two or three concurrent requests are a cheap memory DoS on a single-user machine.
- **Base64 is decoded *after* the limit check, and `Buffer.from` is permissive.** Invalid input does **not** throw — it silently drops junk:
  ```
  Buffer.from('!!!not base64!!!','base64') -> 9e8b5b6ac7ba
  Buffer.from('@@@','base64')              -> length 0
  ```
  So a malformed `audio` produces a garbage/empty buffer that fails deep inside `wavefile` as a 500, not a 400.
- **Non-string `audio` throws a TypeError from `Buffer.from`** (verified for number/null/object/boolean) → caught at line 270 → 500. Note `Buffer.from([1,2,3],'base64')` *succeeds* (arrays are accepted as byte arrays), which is a surprising asymmetry.
- **`JSON.parse` failure and `null` bodies are unhandled.** `JSON.parse('null')` returns `null`, and destructuring throws `TypeError: Cannot destructure property 'audio' of 'd' as it is null.` Malformed JSON throws `SyntaxError`. Both are caught at line 270 and surface as **HTTP 500 with the internal message echoed verbatim**:
  ```
  /%            -> {"status":500,"body":"{\"error\":\"URI malformed\"}"}
  ```
  `server.js:272` — `json(res, 500, { error: String(e.message || e) })` — reflects raw exception text to the client. For a local dev tool this is mostly an information-disclosure nit, but it leaks filesystem paths and library internals, and it means genuine bugs and client mistakes are indistinguishable (both 500).
- **Missing 4xx semantics.** Oversized body → 500, unparseable JSON → 500, wrong type → 500. None of these should be 500.
- **`decodeURIComponent` throws on malformed percent-encoding** (line 264) — `/%`, `/%zz`, `/%E0%A4%A` all raise `URIError: URI malformed` (verified), producing the 500 above instead of a 400.
- **No `Content-Length` pre-check**, so an oversized body is only detected after `size` accumulates.

**Fix.** Check `Content-Length` up front; wrap `JSON.parse` and `decodeURIComponent` in their own try/catch and return 400; validate `typeof audio === 'string'` and its length *before* `Buffer.from`; validate decoded size; strip `error` details from 500s.

---

## 9. LOW — `getAsr` failure path can produce an unhandled rejection

**`server/server.js:37-46`**
```js
function getAsr() {
  if (!asrPromise) {
    asrPromise = pipeline('automatic-speech-recognition', MODEL, { dtype: 'q8', device: 'cpu' })
      .then(p => { console.log('[speaklab] whisper model ready'); return p; })
      .catch(e => { asrPromise = null; throw e; });
  }
  return asrPromise;
}
```

**What's wrong — and what is not.** The caching itself is **correct**: concurrent first requests share one promise and trigger exactly one download. Verified:
```
concurrent calls -> pipeline invocations = 1
after failure asrPromise === null (reset)
```
So the `.catch` reset does allow a retry rather than wedging the server into a permanently-rejected state. Good.

The residual defect is the **unhandled rejection**. The `.catch` re-throws, and the resulting rejected promise is stored in the module-level `asrPromise`. If a caller ever drops it — or if the rejection settles while no consumer is attached — Node emits `unhandledRejection`. Verified:
```
UNHANDLED REJECTION: download failed
```
On Node 15+ the default `--unhandled-rejections=throw` **terminates the process**. The reachable path: `/api/score` (line 259) throws in `decodeAudio` *before* awaiting `getAsr()` at line 62... but `/api/transcribe` (line 250) also decodes first. The realistic trigger is a client aborting mid-request (`req.destroy()` on body-too-large at line 223) after `getAsr()` has been entered, leaving the stored promise unobserved — or any future code path that calls `getAsr()` without awaiting immediately.

Also note **`server.js:245`** — `loaded: !!asrPromise` reports `loaded: true` the instant the promise is *created*, i.e. while the ~40MB download is still in flight (verified: `!!(pendingPromise) === true`). The UI at `index.html:2917` shows "模型已加载" misleadingly.

**Fix.** Keep the cached promise but attach a no-op tail (`asrPromise.catch(() => {})` on a *separate* handle) or track readiness in an explicit state variable (`'idle'|'loading'|'ready'|'error'`) so `loaded` is truthful.

---

## 10. LOW — `env.remoteHost` set from `HF_ENDPOINT` without a trailing slash builds a broken URL

**`server/server.js:27`**
```js
if (process.env.HF_ENDPOINT) env.remoteHost = process.env.HF_ENDPOINT;
```
**`server.js:8`** — `镜像：设置环境变量 HF_ENDPOINT=https://hf-mirror.com 可用国内镜像`

**What's wrong.** transformers.js concatenates `remoteHost + remotePathTemplate` directly, and the built-in default **includes** a trailing slash. Querying the installed library:
```
env.remoteHost         = "https://huggingface.co/"     <-- trailing slash
env.remotePathTemplate = "{model}/resolve/{revision}/"
```
Setting `HF_ENDPOINT` to the documented value (no trailing slash) produces:
```
after server.js:27, env.remoteHost = "https://hf-mirror.com"
RESULTING URL : https://hf-mirror.comXenova/whisper-tiny.en/resolve/main/config.json
CORRECT URL   : https://hf-mirror.com/Xenova/whisper-tiny.en/resolve/main/config.json
```
The host is swallowed into the path — the model download fails, and since this only bites on first run behind the mirror (exactly the case the feature exists for), it manifests as a confusing failure for the users who need it most.

Two related notes: `env.cacheDir` (line 26) is set to `server/.cache`, which is correct and used (confirmed `useFSCache = true`), but it is **never created** and there is no `.gitignore`-visible note in `package.json`; and setting `remoteHost` does **not** disable `env.allowRemoteModels` (still `true`, confirmed), so a user who believes they have pinned a mirror may still reach huggingface.co.

**Fix.** `env.remoteHost = process.env.HF_ENDPOINT.replace(/\/+$/, '') + '/';`

---

## 11. LOW — Stereo down-mix is wrong for more than two channels

**`server/server.js:52-57`**
```js
if (Array.isArray(samples)) { // 立体声 → 混合单声道
  const ch = samples, n = ch[0].length;
  const mono = new Float32Array(n);
  for (let i = 0; i < n; i++) mono[i] = (ch[0][i] + (ch[1] ? ch[1][i] : 0)) / 2;
  samples = mono;
}
```

**What's wrong.** The code averages only channels 0 and 1 but divides by a hard-coded `2` regardless of `ch.length`, and it ignores channels 2..N entirely. For a 4-channel file it sums two channels and halves — producing the average of only 2 of 4 channels, not a down-mix. Measured with a synthetic 4-channel WAV where only channel 0 is non-zero:

```
4ch -> isArray= true channels= 4
server mono (server.js:55)  first 4 = ['500.00000','501.00000','502.00000','503.00000']
true 4ch average            first 4 = ['250.00000','251.00000','252.00000','253.00000']
```
The server's output is **exactly 2x** the correct average — a 6dB gain error, driving Whisper input toward clipping. Also note `wav.getSamples(false, Float32Array)` returns an **unscaled** view for some bit depths, and `wav.fmt.sampleRate` is used at line 50 without validation (feeding finding 6's `Infinity`/`NaN`).

In practice browser `MediaRecorder` from a default `getUserMedia({audio:true})` is usually mono or stereo, so this is low severity — but it is plainly incorrect and trivially fixable.

**Fix.** `mono[i] = sum(ch[c][i] for c in ch) / ch.length`, or better, down-mix explicitly and normalize by peak.

---

## 12. LOW — `words[]` reports the target token while the score describes the spoken token

**`server/server.js:179-182`**
```js
words.push({
  text: a, score, status: score >= .82 ? 'good' : (score >= .6 ? 'mid' : 'bad'),
  refPhon: refPhon || null, saidPhon: saidPhon || null
});
```
where `const a = T[p.a], b = saidWords[p.b]` (line 160).

**What's wrong.** `text` is the *target* word, but `score` was computed from the *spoken* word `b` against `b`'s phonemes. Verified: target `hello world`, spoken `helo world` →

```
said "helo" vs target "hello": words[0].text = "hello"  score = 0.450
simWord(hello,helo) = 1
```

`simWord` returns a perfect `1` (both reduce to phone key `L`), yet the reported score is `0.450` because it takes the `sim >= .75 && a !== b` branch at line 174 (`phoneScore * 0.7`) using CMUdict phonemes for `helo`, which is absent from the dictionary — so `saidPhon` is `null` and line 176 (`score = a === b ? 0.8 : 0.45`) fires. The UI at `index.html:2105` renders `w.text`, so the user is shown the word they were *supposed* to say, annotated with a score derived from something else. When a word is a near-miss, `saidPhon` is `null` (line 181) and the "you said" phoneme display silently disappears — which is exactly the case the phoneme feature exists to explain.

**Fix.** Include both, e.g. `{ text: a, said: b, ... }`, and surface `said` in `adaptServerResult` (`index.html:2102`).

---

## Unconfirmed suspicions

These are plausible but I could not establish them concretely in this environment; they are listed for follow-up rather than as findings.

- **`simWord`/`phoneKey` quality.** `phoneKey` collapses doubled letters (`s.replace(/(.)\1+/g,'$1')`) and truncates to 7 chars, producing collisions I confirmed numerically — `simWord('hello','helo') === 1`, `simWord('th','t') === 1`, `simWord('know','no') === 1`, `phoneKey('queue') === 'K'`. The DP cost at line 129 (`sim >= .75 ? 1.05 - sim : 1.1`) is at least internally consistent with the scoring gate at line 162 (I brute-forced the backtrace over all 0–4 × 0–4 length combinations: **0 non-terminating cases**, so the loop at 136 is safe). Whether the 0.75 threshold causes practically-bad alignments for real English sentences needs real audio; I could not test that without the model and a corpus.
- **`cmudict` entry shape.** `phonemesOf` (line 148) handles both `Array` and `String`. In the installed `cmu-pronouncing-dictionary@3.0.0`, every word I sampled (`route`, `either`, `tomato`, `read`, `the`, `don't`, `hello`) returned a **plain string**, never an array — so the `Array.isArray` branch appears dead. Harmless, but it suggests the code was written against a different version's shape, and I did not enumerate all 130k entries to prove no array entries exist.
- **`word` array/index desync in scoring.** `t.words[p.b]` (line 165) indexes into `t.words` using a `p.b` derived from `saidWords` (line 153). These are the same array today, so it is correct — but it is an implicit coupling with no assertion, and `t.words` is built in `transcribeAudio` (line 70-76) where `.filter(w => w.word)` (line 76) can drop entries. If that filter ever discards a middle element, the indices used here would silently refer to the wrong word. I could not construct a failing input from the current code path.
- **`index.html:2081` 180s timeout vs model download.** `AbortSignal.timeout(180000)` covers the first `/api/score`, which must also download ~40MB. On a slow link the client aborts while the server keeps downloading; combined with finding 9 this is a plausible unhandled-rejection trigger, but I did not reproduce it end-to-end.
- **`sharp`/`onnxruntime-node` native binaries** are present in `node_modules`; I did not audit their supply chain or whether the q8 quantization path matches the `dtype: 'q8'` request at line 41 for `Xenova/whisper-tiny.en` (the comment at line 23 claims this export *does* carry cross-attention, which I did not verify against the actual model files).

---

## Summary

| # | Severity | Location | Issue |
|---|----------|----------|-------|
| 1 | **Critical** | `server.js:266-267` (+`264`) | Prefix escape via `startsWith` + `%2f`/`%5c` decode; sibling file read **proven HTTP 200** |
| 2 | **High** | `server.js:275` | Binds `::` (all interfaces), not 127.0.0.1 as claimed; **LAN-reachable, verified** |
| 3 | **High** | `server.js:47-49` vs `index.html:1213` | WAV-only parser vs WebM/MP4-only client; Whisper engine never works, always 500s |
| 4 | Medium | `server.js:232-237, 242` | `ACAO: *` on all API routes; localhost CSRF/rebinding. Real but bounded — not a credential leak |
| 5 | Medium | `server.js:73-74, 77-79, 170` | `Number(null)===0` → negative last-word duration, `duration=0` |
| 6 | Medium | `server.js:84, 170-174, 199` | `clamp` does not clamp NaN; NaN serializes to `null` in JSON |
| 7 | Medium | `server.js:192-193, 204` | Unparseable target scores a free 0.3; `extras` never penalized |
| 8 | Low/Med | `server.js:218-229, 256-258, 272` | Raw-body limit ≠ memory cap; malformed input → 500 + leaked error text |
| 9 | Low | `server.js:37-46, 245` | Unhandled rejection can kill the process; `loaded` flag lies |
| 10 | Low | `server.js:27` | `HF_ENDPOINT` without trailing slash → `https://hf-mirror.comXenova/...` |
| 11 | Low | `server.js:52-57` | Down-mix ignores channels ≥2 and divides by 2 → 2x gain error on 4ch |
| 12 | Low | `server.js:179-182` | `words[].text` is the target token, score describes the spoken token |
