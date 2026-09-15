# SpeakLab · Voice Studio 🎙️

> A voice studio that lives in your browser: sentence shadowing with per-word pronunciation feedback, three-dimensional scoring and waveform comparison; phoneme gauntlet, scenario conversations, AI follow-up questions; every score is remembered, every day is checked in. Single file, zero dependencies, double-click to run — all practice data stays in your own browser.

> **中文文档：[README.zh-CN.md](README.zh-CN.md)**

![Stack](https://img.shields.io/badge/Stack-Vanilla%20HTML%2FCSS%2FJS-orange)
![Dependencies](https://img.shields.io/badge/Frontend-Zero%20third-party%20libs-brightgreen)
![Form](https://img.shields.io/badge/Form-Single%20file%20index.html-blue)
![Speech](https://img.shields.io/badge/Speech-Web%20Speech%20API-9cf)
![Version](https://img.shields.io/badge/Version-v2.0-lightgrey)

---

## Table of Contents

1. [Introduction & Evolution](#introduction--evolution)
2. [Features](#features)
3. [Quick Start](#quick-start)
4. [Usage Guide](#usage-guide)
5. [Design System](#design-system)
6. [Technical Architecture](#technical-architecture)
7. [Scoring Algorithms](#scoring-algorithms)
8. [Compatibility & Edge Cases](#compatibility--edge-cases)
9. [Privacy](#privacy)
10. [Testing & Verification](#testing--verification)
11. [Development History](#development-history)
12. [Content Library](#content-library)
13. [Project Structure](#project-structure)
14. [Known Limitations](#known-limitations)
15. [Roadmap](#roadmap)
16. [License](#license)

---

## Introduction & Evolution

SpeakLab is an English speaking practice web app for Chinese learners, combining the core gameplay of mainstream speaking products:

| Reference | Borrowed from |
|---|---|
| Liulishuo (英语流利说) | 3-dimension shadowing scores (accuracy/fluency/completeness), per-word red/yellow/green coloring, playback |
| ELSA Speak | Pronunciation feedback visualization, daily short sentences, phoneme courses |
| Speak | AI role-play conversations, in-conversation corrections, listen & repeat |
| Duolingo | Streaks, check-ins, gamified feedback |
| Hibay & similar CN apps | Scenario practice with "hint / translation" helper buttons |
| GitHub: IELTS-Speaking-AI / AI_VoiceCoach / FluentLoop | ASR + scoring + local history; pronunciation analysis & correction |

**Evolution at a glance** (full version-by-version record in [Development History](#development-history)):

```
Shadowing + scenario chat ──► word book / stats / listen&repeat ──► phoneme cards + daily check-in
    ──► per-sentence score memory ──► US/UK accents ──► smart Q&A (never blocks) ──► 3-level corpus
    ──► floating background quotes ──► phoneme gauntlet + auto-score on silence ──► strict scoring system (v2.0)
```

Four training modules + one learning loop:

- **Shadowing Studio**: read a sentence → get scored → per-word color chips + 3-dimension meters + your real waveform compared against the target rhythm; tap any word to hear it slowly; red/yellow words are auto-collected into a mistake word book; every sentence remembers your best/last score.
- **Conversation Studio**: 6 scenarios × two modes: **Smart Q&A** (default — the assistant keeps asking questions; any answer moves the conversation forward) and **Guided Script** (practice line by line); optional listen & repeat; with an LLM key, Smart Q&A upgrades to **AI follow-up questions**.
- **Phoneme Cards**: 44 IPA phonemes — free browsing (pronunciation tips + example words + shadowing check) and a **Gauntlet mode** (increasing difficulty, each gate needs 3 passes to unlock).
- **Stats + Check-in + Memory**: heatmap, score trends, daily goal, check-in window, sentence score book — a complete daily practice loop.

**Browser built-in speech recognition** (zero config, provided by your browser vendor) is the only ASR path: word-level alignment, 3-dimension scoring and phoneme judgment all run on top of its transcript. No model to download, no backend to start.

---

## Features

| Feature | Description |
|---|---|
| 🎙 **Sentence shadowing** | 90-sentence corpus: 5 scenarios (Daily/Travel/Work/Interview/General) × 3 levels (Beginner/Intermediate/Advanced), each with Chinese gloss & pronunciation tip; independent scenario & level filters |
| 🔴🟡🟢 **Per-word coloring** | Word-level alignment between recognition and target; green = good, yellow = close, red = wrong/missed; extra words marked with dashed border |
| 👆 **Tap-to-hear** | Tap any word chip for 0.55× slow pronunciation |
| 📕 **Mistake word book** | Red/yellow words auto-collected (same word accumulates); tap to hear, "re-practice" jumps back to the sentence (auto-switching to its level); removed once read green |
| 📊 **3-dimension scoring (strict)** | Accuracy / Fluency / Completeness + total; level coefficients (Intermediate ×0.97, Advanced ×0.93) |
| 🌊 **Waveform compare stage** | Target sentence rendered as a rhythm spectrum (syllables & stress) vs. your actual recording waveform |
| ▶ **Demo playback** | Browser TTS with word-by-word highlight; **US/UK accent switch** (prefers matching voices, falls back gracefully) |
| 🔁 **Recording & playback** | MediaRecorder + live level meter; 20s cap; phoneme practice auto-scores after ~2s of silence |
| 💬 **Scenario conversations** | 6 scenarios × 2 modes: **Smart Q&A** (8-question bank per scenario, endless follow-ups, any answer advances, hit-rate stats) + **Guided Script** (dialogue tree + fallback guidance + suggestion buttons) |
| 🎙 **Listen & repeat in chat** | Each tutor line can be shadowed & scored inside a modal; score chip on the bubble; can be disabled |
| 🤖 **AI follow-up questions (optional)** | OpenAI-compatible LLM key (DeepSeek/OpenAI/Qwen/GLM presets); interviewer-style system prompt + correction cards + hints + translation |
| 🔤 **Phoneme cards** | 44 IPA phonemes: category browsing, Chinese articulation tips, IPA-highlighted example words with slow playback, shadowing check (word-level judgment), progress |
| 🏆 **Phoneme gauntlet** | 44 gates across 5 difficulty stages; **3 passes unlock the next gate**; progress bar + percentage + road map (passed ✓/current/locked), persisted & resettable |
| 📈 **Stats page** | Streak / total practice / last-30 average + 12-week heatmap + score trend curve + daily goal progress & celebration + sentence score book |
| 📉 **Voice ticker** | A thin always-on bar at the bottom of every page: your live pitch drawn as red/green candlesticks, scrolling right-to-left with off-screen history trimmed. Transparent and background-free so it stays out of the way; independent capture chain, **no backend required** |
| 🦖 **Break-time dino** | The Chromium offline T-Rex, opened as a modal from a home card. Runs only while the modal is open and tears itself down on close, so it never touches your practice session. BSD-3-Clause, see `dino/` |
| 📅 **Daily check-in window** | Auto-pops on the home page; DAY badge reopens it anytime; milestone celebrations at 7/14/30/50/100 days |
| 💬 **Floating background quotes** | 2–3 watermark-style quotes float in empty background space (italic English + author + small Chinese translation), slow drift, staggered rotation; click-through; collapses to one on mobile; static under `prefers-reduced-motion` |
| 🧠 **Sentence score memory** | Every sentence remembers best/last/count (persisted in localStorage); instant comparison after scoring (new record 🎉 / delta vs. last); "sentence score book" in stats (recent/weakest sort) + one-tap re-practice |
| 📅 **Practice history** | Local records of shadowing/chats/phoneme practice; export/import JSON, one-click clear |
| 📱 **Multi-device** | Responsive mobile/tablet/desktop; touch + keyboard operable |
| ♿ **Accessibility & motion** | `:focus-visible` rings, full `prefers-reduced-motion` degradation, `aria-live` announcements |

---

## Quick Start

### Option 1: Double-click (simplest)

Double-click `index.html` and open it in any modern browser (Chrome / Edge / Safari). **No install, no build** (except ASR & LLM features).

Under `file://` the microphone is usable but the browser must be given permission manually, and browsers differ in how well that works (some will not offer it at all). If you run into permission problems, use the local HTTP server in Option 2 — a normal `http://127.0.0.1` origin has far fewer restrictions.

### Option 2: Local HTTP server (recommended; use your phone too)

```bash
# inside the speaklab folder, either:
python -m http.server 8090 --directory .
# or
npx serve -l 8090 .
```

- Desktop: <http://127.0.0.1:8090/index.html>
- Phone (same Wi-Fi): `http://<your-LAN-IP>:8090/index.html`

### Option 3: Local HTTP server on a fixed port

Already covered by Option 2 — `python -m http.server` or `npx serve` both work. Use this when you want a stable origin (e.g. microphone permission remembered across sessions).

### Environment

- Desktop: latest Chrome / Edge; mobile: iOS 15+ Safari / Android Chrome 100+
- ASR is provided by the browser vendor — enabling it sends your audio to that vendor (see [Compatibility](#compatibility--edge-cases) and [Privacy](#privacy))
- No CDNs, no external fonts: the page itself loads no third-party resources
- **Not fully offline**: speech recognition sends your audio to the browser vendor's service, and the optional AI coach sends your text to the LLM provider you configure

---

## Usage Guide

### Shadowing Studio

1. Pick a scenario (Daily/Travel/Work/Interview/General) and a **level** (All/Beginner/Intermediate/Advanced), sentences play in order; the card shows this sentence's **score memory** (times practiced / best / last, or "first attempt").
2. Tap ▶ to hear the demo (word-by-word highlight), then press the red record button and read — live transcript and level meter while recording.
3. Release (or wait for the 20s cap) to get: total + accuracy/fluency/completeness + word chips + waveform compare, plus a comparison against your memory (🎉 new record / ±N vs. last).
4. **Tap any chip to hear that word**; red/yellow words go to the word book (visible on Home), "re-practice" jumps back (auto-switching level), read green to remove.
5. ↺ re-record, ▶ replay, Next sentence. Every score is remembered across sessions.

### Conversation Studio

1. Pick a scenario + a **mode**: 🤖 Smart Q&A (default) or 📖 Guided Script, then Start.
2. **Smart Q&A**: the assistant keeps asking one question after another (8 built-in per scenario, cycling); **whatever you answer, the conversation advances**; matching the suggested answer counts into "answered N · hits M" in the footer; each question has a 中文 translation and a hint.
3. **Guided Script**: progresses line by line; wrong answers get guidance, and two misses reveal clickable suggestions.
4. **Listen & repeat** is on by default: each tutor line has a 🎙 button; shadow it in the modal and the score appears on the bubble.
5. With an LLM key, Smart Q&A becomes **AI follow-up questions**: the AI keeps asking, with correction cards, hints and translations.
6. ⏹ ends the conversation with a summary (mode / turns / hit rate).

### Phoneme Cards

1. Top bar "Phonemes" or the home entry: **📚 Browse** (filter by monophthongs/diphthongs/consonants) or **🏆 Gauntlet**.
2. Browse: tap a card for the articulation tip; tap example words for 0.55× slow demo (target phoneme highlighted in IPA); 🎙 shadowing marks it "practiced".
3. **Gauntlet**: 44 phonemes across 5 difficulty stages, starting from /ɪ/; each gate needs **3 passing shadowings** to unlock the next (🎉 celebration); progress bar, percentage and road map update live; progress persists and can be reset.
4. **One tap is enough**: after speaking, ~2 seconds of silence auto-scores (or tap ■ Stop manually).
5. Judgment: word-level match between the recognized transcript and the target word.

### Daily Check-in Window

1. On the home page, a check-in card pops up ~1.5s after load: streak days + today's practice count; tap 打卡 to check in.
2. It won't auto-pop again that day; the **DAY** badge in the top bar reopens it anytime.
3. Milestones at 7 / 14 / 30 / 50 / 100 consecutive days trigger celebrations.

### Stats Page

Streak / total practice / last-30 average, a 12-week activity heatmap (darker = more), a last-30 score trend curve; the **sentence score book** lists every practiced sentence's best/last/count (recent/weakest sort, one-tap re-practice); set a **daily goal** in Settings and celebrate when reached.

### Settings

- **Demo speed**: 0.6–1.4×; **Accent**: 🇺🇸 US / 🇬🇧 UK (demo, tap-to-hear and phoneme examples all follow; falls back when no matching voice); **Sound effects**: toggle; **Daily goal**: 1–30 sentences.
- **Speech engine**: your browser's built-in speech recognition (no backend, no setup).
- **AI coach**: presets (DeepSeek / OpenAI / Qwen / GLM) or custom base URL + model + key; "Test connection" verifies. The key is stored only in your browser's localStorage.
- **Data**: export/import JSON, clear everything.

---

## Design System

### Concept: a portable recording studio

The page is a recording studio: acoustic-panel texture background, meter tick marks, monospaced readouts, hard-offset shadows — restrained industrial details. The signature element is the **waveform compare stage**: the target sentence's geometric "rhythm spectrum" against your real waveform, making pronunciation gaps visible. The home page adds a **floating quotes** ambience layer: faint watermark-style quotes drift in the empty background, rotating on staggered timers, fully click-through.

### Palette (8 tokens)

| Role | Value | Use |
|---|---|---|
| Warm sandstone | `#F2EEE4` | Page background (acoustic-panel grid texture) |
| Ink green | `#1F4D3E` | Primary, buttons, tutor bubbles, rhythm spectrum |
| Signal red | `#E25B3F` | Record button, emphasis, errors |
| Spectrum green | `#2FA06B` | Good pronunciation |
| Amber | `#E2A23B` | Fair pronunciation |
| Red | `#D9554E` | Bad pronunciation |
| Deep ink | `#22302B` | Body text |
| Sage gray | `#5E7A70` | Secondary text |

### Typography

Zero external fonts (offline-safe): Chinese uses the system sans stack (PingFang SC / MiSans / Microsoft YaHei); Latin display text uses a condensed stack (Arial Narrow / Impact fallback) with uppercase letter-spacing for industrial labels; numbers and meter readouts use monospace (ui-monospace / Consolas).

### Motion

Recording level meter, score count-up, word chips lighting up in sequence, chat bubble entrance, floating quotes drift; everything disabled under `prefers-reduced-motion`.

---

## Technical Architecture

```
speaklab/
├── index.html（single file, ~5,000 lines, organized in commented sections）
│   ├── <style>  design tokens + all styles (top bar / home / shadowing / chat / phonemes / stats / history / settings / modals / check-in card / bg quotes)
│   └── <script> 25 sections:
│       01 utils           02 localStorage (memory fallback)   03 sound FX
│       04 ASR wrapper     05 TTS wrapper (highlight/US/UK)    06 recorder (level/silence)
│       07 waveform render 08 scoring engine (strict)          09 shadowing corpus (90 sentences, 3 levels)
│       10 chat scripts (6 scenarios + 48 Q&A)  10b phoneme data (44 phonemes)  11 history
│       11b mistake word book  11c sentence score memory (derived from history)  12 LLM client
│       13 shadowing module  14 chat module (Q&A/script/AI/listen&repeat)  15 router & top bar
│       16 history/settings views  16b stats view  16c phoneme view (incl. gauntlet)  16d daily check-in
│       16e floating quotes  17 home & init
```

### Key implementations

| Module | Notes |
|---|---|
| ASR | `webkitSpeechRecognition` with dual prefixes; `continuous + interimResults` live transcript; auto-restart on `no-speech/aborted` (max 12); **after stop, final results are still accumulated but no longer pushed to the UI** (prevents stale text written back into the input) |
| TTS | English voices ranked by accent preference (separate US/UK lists); word highlight estimated as `0.24 + 0.075×wordLength / rate` time slices; tap-to-hear at 0.55× |
| Recorder | `MediaRecorder` (webm/mp4 probing) + `AnalyserNode` RMS level & cumulative silence; 20s cap; `decodeAudioData` → PCM waveform; phoneme practice auto-stops after 1.8s of silence |
| Scoring | Word-level Needleman–Wunsch alignment (near-match substitution costs 1.05−sim, always > 0, so **exact matches strictly win**) + light phonetic similarity (consonant skeleton + preserved first vowel) + confidence/rate/silence weighting (see below) |
| Smart Q&A | 8-question bank per scenario cycling + randomized acknowledgments; any answer advances; hit-rate tracking; LLM prompt is an "interviewer that always asks" |
| Mistake word book | Red/yellow words upserted by lowercase form (count accumulates); removed when read green; cap 120; home renders the latest 20 |
| Sentence memory | Derived live from practice history (consistent with export/import/clear); chips on the sentence card + new-record comparison + score book sorting |
| Phoneme gauntlet | 5-stage 44-gate road map; per-gate pass counts persisted; 3 passes unlock the next gate; silence auto-scoring |
| Stats | 12-week × 7-day heatmap (CSS grid, 4 color tiers) + Canvas score trend (mean line + colored endpoints) + daily goal progress |
| LLM chat | OpenAI-compatible fetch from the browser (non-streaming, timeout); prompt demands "reply + JSON(hint/translation/correction)"; graceful errors |
| Storage | `speaklab.*` namespace, history capped at 1000 records; memory fallback in privacy mode |

---

## Scoring Algorithms

> Scores are **heuristic estimates** derived from the recognition transcript, confidence and timing — the honest boundary of a free, backend-free setup. They are tuned to a **strict standard**, and the UI labels the level coefficient.

### Browser engine

1. **Word alignment**: Needleman–Wunsch over tokenized words; substitution cost derived from phonetic similarity (consonant skeleton + preserved first vowel; similarity ≥ 0.75 counts as near, e.g. *coffee↔cofe*); near-match cost is always > 0 so **exact matches strictly take priority**.
2. **Completeness** = matched words / target words.
3. **Accuracy** = mean word score: **exact match tops at 0.9**, near words score similarity ×0.8; recognition confidence weights **0.5+0.5×conf** (a correct reading at confidence 0.6 only gets yellow); missed words score 0. Green ≥ 0.82, yellow ≥ 0.6.
4. **Fluency** = 0.5×rate score (vs. 150 wpm, deviation penalty ×1.6) + 0.3×pause score (penalty ×3.2) + 0.2×hesitation score (−0.25 per um/uh, floor 0.15).
5. **Total** = 0.5×accuracy + 0.3×fluency + 0.2×completeness; **level coefficients**: Intermediate ×0.97, Advanced ×0.93. Total colors at 85/70. In text mode fluency is unavailable → 0.65×accuracy + 0.35×completeness.

### Text mode

When no microphone or recognition is available, type what you read: completeness, accuracy and total all score normally (fluency is unavailable by design, since there is no timing information).

---

## Compatibility & Edge Cases

| Capability | Support |
|---|---|
| ASR | Chrome (Google service, may be unstable behind the GFW) / Edge (Microsoft service, better in CN) / Safari (Siri); Firefox unsupported |
| Recording | MediaRecorder — all modern browsers |
| TTS demo | All browsers; prompts & degrades when no English voice |
| Word highlight | All browsers (estimated time slices, not exact phoneme timing) |

| Situation | Behavior |
|---|---|
| Browser has no ASR / recognition fails | Top warning banner + **text mode** (type what you read — it still scores) |
| ASR network error (common with CN Chrome) | Clear guidance to switch to Edge or text mode |
| Microphone permission denied | Guidance + text mode remains usable |
| Nothing heard / too short | "Didn't catch that, try again" — no score produced |
| LLM key missing / timeout / rate-limited / CORS blocked | Clear error, end conversation and fall back to script mode |
| Page hidden | Recording stops, state resets (timestamp-based, no drift) |
| localStorage unavailable | In-memory fallback — usable but not persistent |
| Extra recognized words | Marked separately as dashed "+word" chips |

---

## Privacy

- ASR audio is transcribed by the browser vendor's service only while you're recording; practice recordings are used **locally** for scoring & playback and are never auto-uploaded.
- LLM requests go directly from your browser to your chosen provider; the key lives only in your browser's localStorage.
- All practice data stays in your browser and can be exported/cleared anytime.

---

## Testing & Verification

> **Note on test code**: the automated test suite described below was written during development but **was never committed to this repository** — `.pwtools/` has been gitignored since the first commit. The behavioural descriptions are kept here as a record of what was verified; **you cannot run them from a clone**. Contributions that bring a runnable test suite into the repo are welcome.

The suite used **playwright-core + system Chrome** (needs the HTTP server running). Tests inject fake SpeechRecognition / speechSynthesis and a fake microphone; LLM calls are mocked via route interception — fully deterministic:

- **Load/home**: title, daily sentence, streak, entry cards, demo playback & meter animation
- **Scoring engine**: perfect reading (completeness 1, accuracy 0.855, total ≈90), complete miss (all red + extras), half reading (6 green 3 red), text-mode total formula, **strictness specifics** (low-confidence correct reading → all yellow, Intermediate ×0.97 / Advanced ×0.93 coefficients)
- **Shadowing loop**: record → live transcript → score → word chips → dual waveform canvas → replay/re-record/next → history written
- **Text mode**: no-ASR degradation, banner, scoring works
- **Guided script chat**: scenario pick, opener, wrong-answer fallback, suggestion buttons after two misses, full walkthrough to summary, history
- **Smart Q&A**: default mode, endless follow-ups, nonsense answers still advance with no suggestion buttons, hit-rate stats, bank cycling, hint, summary & history
- **LLM mode**: intercepted success (reply/translation/correction card/hint) and failure (clear error); interviewer-style system prompt
- **Tap-to-hear & word book**: red-word collection → home list → tap-to-hear → re-practice → auto-removal when green → empty state
- **Stats page**: overview numbers, 84-cell heatmap, trend chart, daily goal progress & celebration, settings linkage
- **Listen & repeat**: bubble button → modal scoring → score chip → history → button disappears when toggled off
- **Chat recording input cleanup**: interim shown while recording, input cleared when stopping without a result, cleared after sending (regression test)
- **Phoneme cards**: 44 cards & filters, articulation tips, IPA highlight, example playback, word-level judgments, practiced marks & progress, history
- **Phoneme gauntlet**: 0/44 start, first gate /ɪ/, 44-dot road map, 3 passes unlock, persistence across reload, reset back to the first gate
- **Silence auto-scoring**: single tap + pause → auto-score counted into the gate
- **Daily check-in window**: auto-popup, state toggle, no repeat same day, DAY badge reopen, 7-day milestone celebration
- **Sentence score memory**: "remembered" first-time note, memory chips, new-record comparison, persistence after reload, score book (count/items/re-practice jump/weakest-first sort)
- **US/UK accents**: default US, switching picks UK voice, correct utterance.lang, persisted, accent note in phoneme detail
- **Shadowing levels**: All/Beginner/Intermediate/Advanced filters, level badge, kept across scene switches & reloads, word-book re-practice auto-switches level
- **Floating quotes**: three slots rendered, click-through, staggered auto-rotation, Chinese translations, static under reduced motion
- **History/settings**: export download, clear, TTS rate slider, LLM presets, save/clear
- **Mobile/edge**: 375px vertical full-flow with zero horizontal overflow, color-token pixel assertions, `prefers-reduced-motion`, `file://` direct open
- **Quality gate**: zero console errors / page errors throughout

Screenshots are kept in `_screenshots/*.png` (home/scoring/chat summary/AI chat/settings/stats/phonemes/gauntlet/mobile etc.).

---

## Development History

The project iterated in "requirements → design → implementation → automated verification → bug fixing" rounds, with a full regression every round and a zero-console-error gate throughout.

| Version | Requirement | Implementation | Verified | Pitfalls fixed |
|---|---|---|---|---|
| **v1.0** initial release | Speaking practice web app: shadowing scoring + scenario chat, benchmarked against Liulishuo/ELSA/Speak | "Recording studio" design system, sentence shadowing + 3-dimension scoring + word chips + waveform compare, script + LLM dual chat modes, 60-sentence corpus + 6 scripts | 83 | ① modal missing its `.modal` wrapper (a real UI bug caught by tests) ② alignment forced unrelated words into "hits" (tightened similarity threshold + post-filter) ③ Chrome `utterance.voice` setter throws on foreign objects ④ favicon 404 caused console errors |
| **v1.1** feature round | Tap-to-hear + word book, stats page, listen & repeat in chat | Word book auto-collect/remove, heatmap + trends + daily goal, modal listen & repeat | 120 | ① practice's scoring thresholds were too loose for short words |
| **v1.2** phoneme cards + check-in | 44 phoneme cards; daily check-in window | Phoneme data (incl. phoneme symbol tables), category browse + shadowing check, check-in card + milestones (7/14/30/50/100) | 143 | ① practice state machine had `busy=true` while recording, blocking the second click → reworked busy semantics |
| **v1.3** sentence score memory | Remember every sentence's score and compare progress | History-derived sentence memory (best/last/count), card chips, new-record/delta feedback, stats score book | 155 | ① README corpus count typo (72 sentences/6 scenes → actually 60/5) |
| **v1.4** US/UK accents | Accent preference for demos | TTS voices ranked by US/UK preference lists, utterance.lang synced, persisted | 162 | — |
| **v1.5** recording input cleanup | Chat input kept leftover recognition text after recording | ASR stops pushing results to UI after stop + stopMic unconditionally clears the input | 166 | ① mic button's own pulse animation made Playwright judge it "unstable" → animation moved to a `::after` ring |
| **v1.6** smart Q&A mode | An assistant that keeps asking questions; off-topic answers still advance | 8-question bank per scenario (48 total) + random acknowledgments + hit-rate stats + cycling; LLM prompt becomes an "interviewer that always asks" | 178 | — |
| **v1.7** shadowing levels | Difficulty tiers for shadowing | 30 new advanced sentences (90 total across 3 levels), level filter bar, level badges, word-book/score-book re-practice auto-switches level | 187 | ① score-book assertions still expected the old corpus size (60→90) |
| **v1.8** floating background quotes | Quotes floating in the empty background | 3 watermark-style floating slots, slow drift + staggered rotation, click-through, mobile collapse, small Chinese translations | 194 | ① the first bar design's hover-pause conflicted with test mouse position → replaced by a fully click-through background layer |
| **v1.9** phoneme gauntlet + silence scoring | Gauntlet (3 passes to unlock); record with a single tap | 5-stage 44-gate road map, per-gate persistence + reset, 1.8s silence auto-scoring | 210 | ① practice's final full re-render wiped the just-shown feedback in gauntlet mode → callback-driven refresh |
| **v2.0** strict scoring system | Scoring was too lenient | Word score cap 0.9, confidence weight 0.5+0.5×conf, tighter fluency penalties, green/yellow lines 0.82/0.6, total colors 85/70, level coefficients ×0.97/×0.93 | 214 | ① phoneKey collapsed all first vowels to 'A' → `a`/`i` got similarity 1 and cross-paired (now preserves the vowel letter) ② `to`/`white` share the same consonant skeleton 'T' → near-substitution cost changed to 1.05−sim so exact matches strictly win |

---

## Content Library

| Content | Size | Notes |
|---|---|---|
| Shadowing corpus | **90 sentences** | 5 scenarios (Daily/Travel/Work/Interview/General) × 3 levels × 6, each with Chinese gloss & pronunciation tip |
| Chat scripts | **6 scenarios × 4–5 turns** | Guided-script mode: accepted replies, hints, translations, fallbacks |
| Smart Q&A bank | **48 questions** | 8 per scenario (question + suggested answer + Chinese translation) |
| Phoneme cards | **44 phonemes** | 12 monophthongs + 8 diphthongs + 24 consonants; each with articulation tip + 2 example words (IPA highlighted) |
| Gauntlet road | **44 gates × 5 stages** | Monophthongs → diphthongs → consonants easy/mid/hard |
| Floating quotes | **24 quotes** | Language/learning themed (English + author + Chinese translation) |

---

## Project Structure

```
speaklab/
├── index.html          # the only frontend deliverable (styles+scripts inline, all content included)
├── README.md           # English documentation (this file)
├── README.zh-CN.md     # 完整中文文档
├── LICENSE             # MIT
├── .gitignore          # excludes .pwtools/ (dev-time test scripts) etc.
├── docs/               # technical assessment, audit reports and a change plan
├── dino/               # BSD-3 notices for the inlined T-Rex Runner (see dino/README.md)
└── _screenshots/       # screenshots
```

> `index.html` inlines the T-Rex Runner as well. Upstream code is BSD-3-Clause,
> so `dino/LICENSE` must stay in the tree — see [dino/README.md](dino/README.md)
> for what was changed and why.

---

## Known Limitations

1. **Scores are heuristic estimates**: recognition text + confidence + rate/pauses can't tell "read correctly but misrecognized" apart from true errors. There is no phoneme-level acoustic analysis.
2. **Recognition accuracy varies**: short words and fast speech may be misrecognized; accuracy depends on the browser vendor's ASR service.
3. **Word highlighting is estimated**: browser TTS provides no precise word timestamps.
4. **CN networks**: Chrome's built-in ASR relies on Google services and may be unstable; Edge generally works better in mainland China.
5. **`file://` differences**: opening `index.html` directly makes the microphone available, but permission behaviour varies by browser; if recording doesn't start, serve the folder over HTTP instead (Quick Start Option 2).
5. **Single-browser storage**: data lives in localStorage with no account system; clearing the browser loses data (export JSON first).
6. **LLM direct calls depend on provider CORS**: some providers block browser direct calls; switch providers or use script mode.

---

## Roadmap

- [ ] Free-speech analysis (Speech-Analyzer style: speak 30–60s freely → rate/pauses/fillers + LLM grammar review)
- [ ] Home "today's tasks" planner (auto-schedules weak shadowing + a chat + phoneme cards)
- [ ] Conversation performance report (grammar error list + idiomatic expressions after LLM chats)
- [ ] Minimal-pair discrimination cards (ship/sheep, bit/beat)
- [ ] Phoneme card recommendations driven by weak phonemes (red words → matching phoneme cards)
- [ ] Spaced-repetition review queue (sentence score memory + SM-2-style scheduling)
- [ ] XP levels + achievement badge wall
- [ ] LLM streaming replies (typewriter effect)
- [ ] PWA (offline cache + add to home screen)

---

## License

MIT License © 2026 — free to use, modify and share. All corpus, chat scripts, phoneme cards and quotes are original content.
