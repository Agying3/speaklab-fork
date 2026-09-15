# SpeakLab frontend JS bug audit

Scope: `index.html` (3547 lines), `server/server.js` (279 lines). All line numbers refer to `index.html` unless prefixed.

Verdicts are marked **[CONFIRMED]** (verified by reading the cited code and tracing every caller) or **[PARTIAL]**.
I re-checked and *dropped* several plausible-looking bugs; see "Checked and NOT a bug" at the end, plus the SUSPECTED list.

---

## CRITICAL

### 1. [CONFIRMED] Clicking "下一句 / 重录 / a scene chip" while recording or analyzing never stops the microphone, and the score is written to the wrong sentence

**Where:** `reset()` lines 2220–2233; `#btnNext` handler line 2151; `renderScore()` line 2318 / 2355 / 2383.

```js
2151:  $('#btnNext').addEventListener('click',()=>{ this.idx=(this.idx+1)%this.list().length; this.reset(); this.renderSentence(); });
```

```js
2220:  reset(){
2221:    TTS.stop();
2222:    this.phase='ready'; this.result=null; this.asrFailed=false;
2223:    if(this.blobUrl){ URL.revokeObjectURL(this.blobUrl); this.blobUrl=null; }
```

**What's wrong — two separate defects in one path.**

(a) `#btnNext` (HTML line 748) is *always visible* and its handler has **no phase guard**, unlike every sibling handler: scene chips check `if(this.phase==='recording') return;` (line 2175), level chips check the same (line 2186), and `playDemo()` checks the same (line 2236). So "下一句" is clickable during `'recording'` *and* during `'processing'`.

(b) `reset()` never stops the recorder and never clears `this.recTimerId` / `this.autoStopId`. It sets `phase='ready'` only.

**Concrete user-visible symptoms:**

- **Mic leak (during recording):** start recording, then click "下一句 →". `phase` becomes `'ready'` while `Recorder.state` is still `'rec'` and `ASR` is still open. The 20 s auto-stop guard at line 2264 is `if(this.phase==='recording')` — now false — so it silently does nothing, and `ASR`/`MediaRecorder` run **forever**. The browser's recording indicator stays lit and the level meter keeps moving. Also `$('#recTimer')` was reset to `'00:00'` at line 2231 but `recTimerId` was never cleared, so the timer immediately resumes climbing from the old `recStartTs`.
- **Score attributed to the wrong sentence (during processing):** `stopRecording()` yields at `await Recorder.stop()` (2281) and again at `await voiceScore(...)` (2293). Click "下一句" in that window: `this.idx` advances, so every subsequent `this.cur()` call inside `renderScore()` resolves to the **new** sentence:
  - `2318: const prev=SentenceMemory.get(this.cur().en);` → memory comparison is against the wrong sentence
  - `2355: Mistakes.addFromResult(r, this.scene, this.cur().en, this.cur().zh)` → word-book entry cites the wrong source sentence
  - `2383: History.add({..., en:this.cur().en, ...})` → **sentence B is permanently credited with sentence A's score**, which also corrupts `SentenceMemory` for both sentences and the stats "sentence score book"

  The UI simultaneously shows sentence B's text with sentence A's score panel, chips and waveform.

**How to reproduce:** Shadow tab → click the red record button → while recording, click "下一句 →". Watch the mic indicator stay on and the timer keep counting. For the mis-attribution: record, then click "下一句 →" during "ANALYZING · 分析中…" and compare the recorded score in 统计 → 句子成绩本 against the sentence shown.

**Confidence:** high. The guard asymmetry with lines 2175/2186/2236 is unambiguous.

---

### 2. [CONFIRMED] `ChatApp.finish()` silently discards the last spoken answer; the same modal-close path leaks the recorder

**Where:** `finish()` lines 2773–2777, `stopMic()` lines 2761–2772, `sendUser()` line 2648.

```js
2773:  finish(silent){
2774:    if(this.ended) return;
2775:    this.ended=true;
2776:    TTS.stop();
2777:    if(this.recOn) this.stopMic();
```

```js
2761:  async stopMic(){
2765:    await Recorder.stop();
2767:    const t=(ASR.final||'').trim();
2769:    $('#chatInput').value='';
2770:    if(t){ this.sendUser(t); }
```

```js
2646:  async sendUser(text){
2648:    if(!text || this.busy || this.ended) return;
```

**What's wrong:** `stopMic()` is `async`; its body runs synchronously only as far as `await Recorder.stop()` (2765). Lines 2767–2770 therefore execute **after** `finish()` has already returned, at which point `this.ended === true` (set at 2775). `sendUser()` then hits the `this.ended` early-return at 2648 and drops the text. The transcript is read *and thrown away*.

**Concrete user-visible symptom:** talk into the mic in the conversation, then click "⏹ 结束对话" (or switch scenes / re-practice via the summary modal) without first pressing the mic button to stop. Your last turn disappears entirely: it never appears as a user bubble, `turns` is not incremented, and it is not sent to the LLM.

**How to reproduce:** Chat tab → start a conversation → click 🎤 → say a sentence → without clicking 🎤 again, click "⏹ 结束对话". The final utterance is never added to the log.

**Confidence:** high. I initially mis-stated the mechanism (I claimed `ASR.final` was clobbered); a verification pass corrected me — `ASR.stop()` is synchronous and `final` persists by design — but the `ended` guard at 2648 conclusively drops the send, because 2770 runs on the far side of the 2765 await.

---

### 3. [CONFIRMED] The "listen & repeat" and phoneme-practice modals leak an open microphone when dismissed any way other than the happy path

**Where:** `ChatApp.openShadow` line 2638; `PhoneticsView.openDetail` line 3263; `PhoneticsView.practice` lines 3269–3296.

```js
2638:    $('#msCancel').addEventListener('click',closeModal);
```

```js
3263:    $('#phClose').addEventListener('click',closeModal);
```

**What's wrong:** both modals start `Recorder.start()` + `ASR.start()` (lines 2592/2599 and 3278/3290), but the only close handlers are bare `closeModal`, which just does `$('#modalRoot').innerHTML=''` (line 1043). There is **no `Recorder.stop()`, no `Recorder.cleanup()` and no `ASR.stop()`** on the cancel path. Escape and backdrop-click (lines 1038–1039) route to the same bare `closeModal`.

`PhoneticsView` is strictly worse because the state is on the singleton object, not in a closure: `practice()` sets `this.recOn = true` (line 3285) and only the *stop* branch (line 3299) sets it back to `false`. Closing the modal mid-recording leaves `PhoneticsView.recOn === true` forever.

**Concrete user-visible symptoms:**
- Microphone stays open (browser recording indicator lit) after closing either modal with 取消 / 关闭 / Esc / clicking outside.
- For phoneme cards, the *next* time you click 🎙 跟读 on any card, `practice()` sees `this.recOn === true` and jumps straight to line 3296's "stop" branch. It calls `Recorder.stop()` on the **leaked** session, then scores whatever stale transcript `ASR.final` holds. The card flashes "跟读完成" / "没听清" instantly without you ever speaking again, and the phoneme-history entry written at line 3323 is bogus. You have to click 跟读 a second time before a real recording starts.

**How to reproduce:** Chat → 🎙 跟读 on any tutor line → click ● 录音 → immediately click 取消. Observe the mic indicator. For the phoneme half: 音标 → Gauntlet → 🎙 跟读 → 关闭 while recording → then click 🎙 跟读 on the next card and watch it finish instantly without recording.

**Confidence:** high.

---

## MAJOR

### 4. [CONFIRMED] `Engine.mode()`'s default `'auto'` is dead code — the documented "same-origin auto-uses /api" behaviour does not exist

**Where:** `Engine.mode()` line 2067; the only three consumers, lines 2113, 2292, 3306.

```js
2067:  mode(){ return Store.get('engine','auto'); },
```

```js
2113:  if(Engine.mode()==='whisper' && rec.blob){
```

```js
2292:    if(Engine.mode()==='whisper' && r.blob) this.setStatus('WHISPER · 本地模型转写中…');
```

```js
3306:    if(Engine.mode()==='whisper' && rec.blob) fbEl.textContent='本地模型转写中…';
```

**What's wrong:** every consumer tests `=== 'whisper'`. The default value is `'auto'`, and **no code anywhere handles `'auto'`** — I grepped the whole file: the string `'auto'` appears only at the `<option value="auto">` (line 953) and the default in `mode()`. There is no health probe, no same-origin detection, no automatic engine selection.

**Concrete user-visible symptom:** a user follows README §"Option 3" (README lines 115–127: *"Open `http://127.0.0.1:8091/index.html` (same origin auto-uses `/api`)"*), starts the Whisper server, opens the served page, and gets **browser ASR only**. The Whisper engine is silently never used, no error, no toast — until they discover 设置 → 语音引擎 → select "本地 Whisper" → 保存. The README's central selling point for the `server/` option is simply not implemented.

**How to reproduce:** start `server/` (`npm start`), open `http://127.0.0.1:8091/index.html`, record a sentence while watching the Network tab: no request to `/api/score` is ever made. Then set the engine to 本地 Whisper in Settings and repeat — the request appears.

**Confidence:** high (grep-verified absence of any `'auto'` branch).

---

### 5. [CONFIRMED] Double-tapping the chat mic button leaves the microphone open permanently

**Where:** `toggleMic()` line 2745–2760, `Recorder.start()` lines 1210–1240, `Recorder.stop()` line 1243.

```js
2752:    Recorder.start({}).catch(e=>{          // ← not awaited
```

```js
1210:  async start({onLevel}={}){
1211:    this.stream = await navigator.mediaDevices.getUserMedia({audio:true});
1212:    ...
1236:      this.state='rec';
```

```js
1243:      if(this.state!=='rec'){ res({blob:null,durMs:0,silMs:0,mime:''}); return; }
```

**What's wrong:** `Recorder.start()` does not set `state='rec'` until **after** the `await getUserMedia` on line 1211 (the assignments are at 1236/1238). `toggleMic` fires `Recorder.start({})` without awaiting it, and `stopMic()` does `await Recorder.stop()`.

Interleaving: click 🎤 → `recOn=true` (2749) → `start()` suspends at the permission await. Click 🎤 again before permission resolves → `toggleMic` sees `recOn===true` → `stopMic()` → `ASR.stop()` (no-op, `rec` is null) → `await Recorder.stop()` → `state==='idle'` → **early return at 1243, so `cleanup()` never runs**. The first click's `start()` then resumes, acquires the stream, calls `this.rec.start(250)` and sets `state='rec'`.

**Concrete user-visible symptom:** the microphone permission prompt appears twice / the recording indicator stays lit and the recorder runs indefinitely. Pressing 🎤 again does **not** fix it: it sees `recOn===false` and calls `start()` a *second* time, opening a second stream on top of the first. The only recovery paths are the `visibilitychange` handler (line 3533) or ending the conversation (line 2777). Nothing in `toggleMic` can stop the leaked session.

**How to reproduce:** Chat tab → start a conversation → double-click 🎤 quickly (first click triggers the permission prompt). Answer the prompt. The mic stays hot and the button is not in the recording state.

**Confidence:** high. `state` is only ever read at line 1226 and 1243, and there is no `stopRequested` flag or generation token anywhere.

---

### 6. [CONFIRMED] Repeatedly-failed words are the *first* to be evicted from the word book

**Where:** `Mistakes.addFromResult` lines 1923–1933, `renderMistakes` line 1953.

```js
1926:        const f=list.find(m=>m.word===key);
1927:        if(f){ f.count++; f.ts=Date.now(); }
1928:        else { list.push({word:key, en, zh, scene, count:1, ts:Date.now()}); added++; }
...
1931:    if(list.length>120) list.splice(0,list.length-120);
```

**What's wrong:** `list` is in **insertion** order. Re-encountering an existing word bumps its `count` and `ts` in place (line 1927) **without moving it**, so insertion order and recency order diverge. The cap at line 1931 evicts from the front — i.e. by insertion order. Meanwhile `renderMistakes` sorts by `ts` descending (line 1953), so the UI presents the list as "most recently seen".

**Concrete user-visible symptom:** once the book exceeds 120 distinct words, the words you keep getting wrong are deleted first (they have the oldest insertion positions) while one-off words you already stopped missing are retained. The "收录 N 次" counter that represents your hardest words is exactly what gets dropped, and 重练 targets vanish. This directly contradicts README line 242 ("same word accumulates") / line 76 ("red/yellow words auto-collected").

**How to reproduce:** in devtools, `JSON.parse(localStorage.getItem('speaklab.mistakes'))`, then call `Mistakes.addFromResult` with 130 synthetic results; note that the entry with the highest `count` is evicted.

**Confidence:** high — the divergence between the eviction key (array position) and the display key (`ts`) is unambiguous.

---

### 7. [CONFIRMED] `jumpTo()` with a sentence not in the corpus throws and bricks the shadow tab

**Where:** `ShadowApp.jumpTo()` lines 2391–2401, `cur()` line 2133, `renderSentence()` lines 2195–2197, producer at line 1996.

```js
2391:  jumpTo(scene, en){
2392:    const s=SENTENCES.find(x=>x.en===en);
2393:    this.scene=scene;
2395:    this.level = s ? s.lv : 'all';
2397:    const list=this.list();
2398:    const i=list.findIndex(x=>x.en===en);
2400:    this.renderSceneBar(); this.renderLevelBar(); this.reset(); this.renderSentence();
```

```js
2132:  list(){ return SENTENCES.filter(s=>s.scene===this.scene && (this.level==='all' || s.lv===this.level)); },
2133:  cur(){ return this.list()[this.idx]; },
```

```js
2196:    const s=this.cur();
2197:    $('#shadowSceneTag').textContent=SCENE_NAMES[s.scene]+' · '+LV_NAMES[s.lv];
```

```js
1996:      .map(en=>({en, scene:(byEn[en]||{}).scene||'', zh:(byEn[en]||{}).zh||'', lv:(byEn[en]||{}).lv||null, ...map[en]}))
```

**What's wrong:** `s` is `undefined` for an unknown `en`, and `this.level` falls back to `'all'` — but `this.scene` is set to the **caller-supplied** `scene`, which `SentenceMemory.list()` defaults to `''` for any `en` not found in `SENTENCES` (line 1996). `list()` then filters on `s.scene===''` → `[]` → `cur()` → `undefined` → line 2197 throws `TypeError: Cannot read properties of undefined (reading 'scene')`.

**Concrete user-visible symptom:** on 统计 → 句子成绩本, clicking 重练 on any entry whose sentence is not in the current corpus throws, the shadow tab opens with a **stale sentence card**, all scene chips unselected, and no working status. Recording then throws again at line 2293 (`this.cur().en`). The tab is unusable until the user clicks a scene chip.

**How to reach it:** import a history JSON (an advertised feature, README line 91/179) that contains a `type:'shadow'` record for an `en` absent from `SENTENCES` — e.g. a file exported before the corpus changed, or a hand-edited file. `SentenceMemory.derive()` picks it up at line 1981 and `list()` renders a 重练 button with `data-scene=""`.

**Confidence:** high for the crash; the trigger requires an imported/foreign history file (normal in-app use always records `this.cur().en`, line 2383).

---

## MINOR

### 8. [CONFIRMED] `#ciToday` is never updated; `Checkin.render()` writes the whole sentence into the streak `<b>`

**Where:** line 3366 vs HTML line 1010.

```js
3366:    $('#ciStreak').innerHTML='连续打卡 <b>'+this.streak()+'</b> 天 · 今日已练 <b>'+this.todayShadow()+'</b> 句';
```

```html
1010:  <div class="ci-streak">连续打卡 <b id="ciStreak">0</b> 天 · 今日已练 <b id="ciToday">0</b> 句</div>
```

**What's wrong:** `#ciStreak` is the inner `<b>` holding only the *number*, and `#ciToday` is a separate sibling `<b>`. Grepping the whole file, `ciToday` occurs **only** in the static HTML at line 1010 — no script ever reads or writes it. Line 3366 injects the entire sentence as markup *inside* the streak `<b>`.

**Concrete user-visible symptom:** the check-in card renders the sentence twice — once from the HTML text nodes and once from the injected string — with nested `<b><b>7</b></b>`, and "今日已练 **0** 句" is frozen at `0` no matter how many sentences you practice. `todayShadow()` is computed correctly (lines 3361–3364) but lands in the wrong element.

**How to reproduce:** practice a few sentences, click the **DAY** badge in the top bar: the card shows the doubled sentence and 0.

**Confidence:** high (grep-verified: exactly 4 hits for `ci-streak|ciStreak|ciToday`, one of which is CSS).

---

### 9. [CONFIRMED] `todaySentence()` off-by-one — "Today's line" is always one ahead, and sentence #1 only appears on day 90

**Where:** lines 3463–3467.

```js
3464:    const now=new Date();
3465:    const start=new Date(now.getFullYear(),0,0);   // Dec 31 of the previous year
3466:    const doy=Math.floor((now-start)/86400000);    // 1-based day of year
3467:    return SENTENCES[doy % SENTENCES.length];
```

**What's wrong:** `new Date(y,0,0)` is Dec 31 of year `y-1`, so `doy` is the **1-based** day of year (Jan 1 → `1`). Indexing with `doy % len` therefore starts at index 1, not 0. `SENTENCES[0]` ("Could I have a flat white to go, please?") is reachable only when `doy % 90 === 0`, i.e. day 90/180/270/360.

**Concrete user-visible symptom:** the home page's "今日一句" never shows the first corpus sentence on a day-boundary-consistent cycle; the rotation is shifted by one for the entire year. Cosmetic but a genuine off-by-one — line 3467 should be `SENTENCES[(doy-1) % SENTENCES.length]`.

**Confidence:** high on the arithmetic. (Note: the `86400000` ms-per-day assumption would also drift across a DST transition, but China observes no DST, so it is inert for the target audience — see SUSPECTED.)

---

### 10. [CONFIRMED] `openModal()` leaks a `document` keydown listener on every non-Escape close

**Where:** lines 1034–1043.

```js
1039:    const escH=e=>{ if(e.key==='Escape'){ closeModal(); document.removeEventListener('keydown',escH); } };
1040:    document.addEventListener('keydown',escH);
...
1043:  function closeModal(){ $('#modalRoot').innerHTML=''; }
```

**What's wrong:** `escH` removes itself only when it actually observes an Escape key. `closeModal()` — reached from every 关闭 / 取消 / 完成 / 背景点击 path (lines 2638, 2639, 2797, 2798, 2799, 2990, 2991, 3139, 3140, 3263) — does not remove it. Each such close permanently adds one handler to `document`.

**Concrete user-visible symptom:** modest but real — handlers accumulate without bound for the session. After closing a modal with a button, pressing Escape runs the stale handler, which calls `closeModal()` again; if a *different* modal is open, the stale handler closes that one too, so Escape still works but the listener list grows unboundedly (a real leak in a long session, e.g. many listen-&-repeat rounds).

**Confidence:** high.

---

### 11. [CONFIRMED] `LLM.parse()` strips text greedily, and strips even when the JSON parse failed

**Where:** lines 2044–2050.

```js
2046:    const m=String(text).match(/\{[\s\S]*?"hint"[\s\S]*?\}/);
2047:    if(m){ try{ const j=JSON.parse(m[0]); meta={...}; }catch(e){} }
2048:    const reply=String(text).replace(/\{[\s\S]*\}/,'').trim();
```

**What's wrong:** line 2048's pattern is greedy — `[\s\S]*` spans from the **first** `{` in the reply to the **last** `}`. Any brace-preceded content (e.g. a reply containing `{...}` or a stray `}` late in the text) causes everything between them to be deleted from the visible reply. The strip also runs unconditionally, so when the parse at 2047 **fails** (line 2047's `catch` swallows it), `meta` is all-empty *and* the reply is still truncated — the user loses both the correction card and part of the answer, with no error surfaced.

**Concrete user-visible symptom:** AI turns occasionally come back with the middle of the reply missing and no hint / translation / correction card attached, with no error message telling the user anything went wrong.

**Confidence:** medium-high (code is unambiguous; the practical frequency depends on the model's formatting).

---

## Checked and NOT a bug (dropped after verification)

- **`Recorder.stop()` promise hang / `Recorder.cancel()`** (lines 1256, 1241–1255): `cancel()` is **dead code** — the only `.cancel()` call in the file is `speechSynthesis.cancel()` at line 1169. Even if invoked, `cleanup()` never nulls `this.rec`, so the queued `onstop` still fires and the promise resolves. Not a bug.
- **`reset()` leaving `#stageStatus` hidden** (2233 vs 2260): `stopRecording()` unconditionally does `$('#stageStatus').classList.remove('hidden')` (line 2276), so the next stop always restores it. Not a bug on the normal path.
- **`shadowLevel` stored as number vs `'all'` string** (2140/2187/2188/2183/2132): every read path uses loose comparison (`s.lv===this.level` where both are numbers) or `String(this.level)===o.k`. Consistently handled. Not a bug.
- **`PhoneticsView.practice` busy-flag re-entrancy** (3266, 3272–3275, 3298, 3325): the `autoFired` guard plus `cancelAnimationFrame` inside `Recorder.stop()` prevent the auto-stop recursion from double-firing, and `busy` is reset on both the success and no-result branches. Not a bug.
- **`recOn`/gate re-render wiping feedback** (3321, 3327): `onPass` is deferred by 700 ms and `busy` is already false, so the feedback is visible before `renderGate()` re-renders. Working as the comment claims.
- **`ChatApp.finish()` when `sc` is null**: `finish()` is only reachable after `start()`, which returns early without `sc`; `backToSetup()` is only wired from the summary modal (line 2798). Not reachable.

---

## SUSPECTED but not confirmed

1. **ASR restart can die permanently.** `ASR.open()` line 1130 wraps `r.start()` in a try/catch that reports errors, but `tryRestart()` line 1132 swallows failures: `try{ this.rec.start(); }catch(e){}`. If Chrome throws `InvalidStateError` ("recognition has already started", common when an `onend` follows a `start()` closely), the catch is empty, `this.started` stays `true`, and no further `onend` fires to trigger another restart — recognition is silently dead for the rest of the session and the user only ever sees "没听清". **Could not confirm** without a live browser: the restart cadence depends on Chrome's internal state machine, and I could not determine how often `start()` actually throws here.
2. **`ASR.open()` is never called again after `stop()`.** `start()` resets counters and calls `open()`, so a fresh `SR` instance is built per session — but the previous instance's `onresult`/`onend` handlers are never detached; they still reference `this` (shared state). If a late `onend` from the previous session arrives after a new session began, line 1128's `if(this.started && !this.stopFlag)` now refers to the *new* session and triggers a spurious `tryRestart()`. **Could not confirm** the timing is achievable in practice.
3. **Whisper `targetWords` fallback uses the transcript.** `adaptServerResult` line 2104: `targetWords: sr.targetWords || tokenize(sr.text||'')`. If a future/degraded server response omits `targetWords`, the "TARGET" prosody spectrum (line 2377) would be drawn from *your own* recognized text instead of the target sentence. The bundled `server.js` always returns `targetWords` (line 213), so this is currently latent. **Could not confirm** it is reachable with the shipped server.
4. **History import cap can delete imported records.** Line 2974 puts imported records *first* (`arr.filter(...).concat(cur)`), while `History.add` (line 1901–1902) trims from the **front** past 1000. Importing into a non-empty history can therefore push the total over 1000, and the next recorded practice deletes the newly-imported (oldest-position) records. I traced the arithmetic but **could not confirm** a realistic user hits 1000 records; also `SentenceMemory.derive()` reads in array order rather than by `ts`, which is only accidentally correct here.
5. **`file://` double-click support is weaker than advertised.** README line 3/99 promises "double-click to open". Under `file://`, Chrome provides neither `navigator.mediaDevices` (so `Recorder.supported()` is false → text mode) nor a usable Web Speech origin, and `Engine.health('')` (line 2072) resolves `/api/health` against `file:///api/health`. The code *does* degrade to text mode as documented, so I could not confirm an outright failure — but I could not verify the Web Speech API's behaviour on a `file://` origin without a browser, and the app's own `initBanner()` will report "不支持语音识别" there.
6. **`FX.unlock()` AudioContext is never `close()`d.** Line 1062 creates one shared `AudioContext` and reuses it; `Recorder` keeps a second one (line 1218). Neither is ever closed. Browsers cap concurrent AudioContexts per page (~6 in Chrome), but only two are ever created here, so I could not confirm a real exhaustion path.
