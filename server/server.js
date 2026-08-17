/* SpeakLab · 本地 Whisper 引擎
   - 静态托管 index.html（同源 /api）
   - POST /api/transcribe   音频 → Whisper 转写（词级时间戳）
   - POST /api/score        音频 + 目标句 → 音素词典对比 + 评分
   - GET  /api/health       引擎状态

   模型：onnx-community/whisper-tiny.en（int8 量化，约 40MB，首次自动下载）
   镜像：设置环境变量 HF_ENDPOINT=https://hf-mirror.com 可用国内镜像
   运行：npm install && npm start   → http://127.0.0.1:8091
*/
import http from 'node:http';
import { createReadStream, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { pipeline, env } from '@huggingface/transformers';
import wavefilePkg from 'wavefile';
import { dictionary as cmudict } from 'cmu-pronouncing-dictionary';
const { WaveFile } = wavefilePkg;

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');
const PORT = Number(process.env.PORT || 8091);
/* Xenova 导出带 cross-attention，支持词级时间戳（onnx-community 导出不带，无法出词时间戳） */
const MODEL = process.env.SPEAKLAB_MODEL || 'Xenova/whisper-tiny.en';

env.cacheDir = path.join(__dirname, '.cache');
if (process.env.HF_ENDPOINT) env.remoteHost = process.env.HF_ENDPOINT;

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.png': 'image/png', '.jpg': 'image/jpeg',
  '.json': 'application/json', '.svg': 'image/svg+xml', '.ico': 'image/x-icon',
  '.webm': 'audio/webm', '.mp4': 'audio/mp4', '.woff2': 'font/woff2'
};

/* ---------------- Whisper ---------------- */
let asrPromise = null;
function getAsr() {
  if (!asrPromise) {
    console.log('[speaklab] loading whisper model:', MODEL, '(first run downloads ~40MB)');
    asrPromise = pipeline('automatic-speech-recognition', MODEL, { dtype: 'q8', device: 'cpu' })
      .then(p => { console.log('[speaklab] whisper model ready'); return p; })
      .catch(e => { asrPromise = null; throw e; });
  }
  return asrPromise;
}
function decodeAudio(buf) {
  const wav = new WaveFile();
  wav.fromBuffer(buf);
  const sr = wav.fmt.sampleRate;
  let samples = wav.getSamples(false, Float32Array);
  if (Array.isArray(samples)) { // 立体声 → 混合单声道
    const ch = samples, n = ch[0].length;
    const mono = new Float32Array(n);
    for (let i = 0; i < n; i++) mono[i] = (ch[0][i] + (ch[1] ? ch[1][i] : 0)) / 2;
    samples = mono;
  }
  return { samples, sr };
}
async function transcribeAudio(buf) {
  const { samples, sr } = decodeAudio(buf);
  const asr = await getAsr();
  let out;
  try {
    out = await asr(samples, { return_timestamps: 'word', chunk_length_s: 30, stride_length_s: 5 });
  } catch (e) {
    console.warn('[speaklab] 词级时间戳不可用，降级为整句转写:', e.message);
    out = await asr(samples, { chunk_length_s: 30, stride_length_s: 5 });
  }
  const words = (out.chunks || [])
    .map(c => ({
      word: String(c.text || '').trim(),
      start: c.timestamp ? Number(c.timestamp[0]) : null,
      end: c.timestamp ? Number(c.timestamp[1]) : null
    }))
    .filter(w => w.word);
  const duration = words.length && words[words.length - 1].end != null
    ? words[words.length - 1].end
    : (samples.length / sr);
  return { text: String(out.text || '').trim(), words, duration };
}

/* ---------------- 评分（词对齐 + 音素词典对比） ---------------- */
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const tokenize = s => (String(s).toLowerCase().replace(/[’‘]/g, "'").match(/[a-z]+(?:'[a-z]+)*/g) || []);
function levenshtein(a, b) {
  if (a === b) return 0;
  const m = a.length, n = b.length;
  if (!m) return n; if (!n) return m;
  let prev = new Array(n + 1), cur = new Array(n + 1);
  for (let j = 0; j <= n; j++) prev[j] = j;
  for (let i = 1; i <= m; i++) {
    cur[0] = i;
    for (let j = 1; j <= n; j++) cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    [prev, cur] = [cur, prev];
  }
  return prev[n];
}
function phoneKey(w) {
  let s = String(w).toUpperCase().replace(/[^A-Z]/g, '');
  if (!s) return '';
  s = s.replace(/^KN/, 'N').replace(/^GN/, 'N').replace(/^PN/, 'N').replace(/^WR/, 'R').replace(/^WH/, 'W');
  s = s.replace(/(.)\1+/g, '$1');
  let out = /^[AEIOU]/.test(s) ? s[0] : '';
  s = s.replace(/[AEIOU]/g, '');
  out += s.replace(/CK|Q|X/g, 'K').replace(/PH/g, 'F').replace(/GH/g, '')
    .replace(/TH/g, 'T').replace(/CH|SH/g, 'H').replace(/J|G/g, 'J')
    .replace(/V/g, 'F').replace(/Z/g, 'S').replace(/B/g, 'P').replace(/D/g, 'T').replace(/W|H|Y/g, '');
  return out.slice(0, 7);
}
function simWord(a, b) {
  a = String(a).toLowerCase(); b = String(b).toLowerCase();
  if (a === b) return 1;
  const ka = phoneKey(a), kb = phoneKey(b);
  if (!ka || !kb) return 0;
  const d = levenshtein(ka, kb);
  return Math.max(0, 1 - d / Math.max(ka.length, kb.length));
}
function alignWords(A, B) {
  const n = A.length, m = B.length;
  const dp = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  const bt = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(''));
  for (let i = 0; i <= n; i++) { dp[i][0] = i; bt[i][0] = 'up'; }
  for (let j = 0; j <= m; j++) { dp[0][j] = j; bt[0][j] = 'left'; }
  bt[0][0] = '';
  for (let i = 1; i <= n; i++) for (let j = 1; j <= m; j++) {
    const sim = simWord(A[i - 1], B[j - 1]);
    /* 代价 1.05-sim 恒大于 0：精确匹配严格优先于音近交叉配对 */
    const sub = dp[i - 1][j - 1] + (A[i - 1] === B[j - 1] ? 0 : (sim >= .75 ? 1.05 - sim : 1.1));
    const del = dp[i - 1][j] + 1, ins = dp[i][j - 1] + 1;
    if (sub <= del && sub <= ins) { dp[i][j] = sub; bt[i][j] = 'diag'; }
    else if (del <= ins) { dp[i][j] = del; bt[i][j] = 'up'; }
    else { dp[i][j] = ins; bt[i][j] = 'left'; }
  }
  const pairs = []; let i = n, j = m;
  while (i > 0 || j > 0) {
    if (bt[i][j] === 'diag') { pairs.unshift({ a: i - 1, b: j - 1 }); i--; j--; }
    else if (bt[i][j] === 'up') { pairs.unshift({ a: i - 1, b: null }); i--; }
    else { pairs.unshift({ a: null, b: j - 1 }); j--; }
  }
  return pairs;
}
/* 发音词典（CMUdict，13 万词） */
function phonemesOf(word) {
  const w = String(word).toLowerCase().replace(/[^a-z']/g, '');
  const entry = cmudict[w];
  if (!entry) return null;
  const s = Array.isArray(entry) ? entry[0] : String(entry);
  return s.split(/\s+/).map(p => p.replace(/\d/g, ''));
}
function scoreResult(target, t, level=1) {
  const T = tokenize(target);
  const saidWords = t.words.map(w => w.word);
  const pairs = alignWords(T, saidWords);
  const words = []; const usedB = new Set();
  let matched = 0, accSum = 0;
  for (const p of pairs) {
    if (p.a === null) continue;
    if (p.b !== null) {
      const a = T[p.a], b = saidWords[p.b];
      const sim = simWord(a, b);
      if (a === b || sim >= .75) {
        usedB.add(p.b);
        const refPhon = phonemesOf(a), saidPhon = phonemesOf(b);
        const wInfo = t.words[p.b];
        let score;
        if (refPhon && saidPhon) {
          const d = levenshtein(refPhon, saidPhon);
          const phoneScore = Math.max(0, 1 - d / Math.max(refPhon.length, saidPhon.length));
          const dur = (wInfo.end != null && wInfo.start != null) ? (wInfo.end - wInfo.start) : 0.4;
          const expected = 0.18 + refPhon.length * 0.09;
          /* 严格：时长偏离惩罚 ×1.6，时长权重提到 0.3 */
          const durScore = clamp(1 - Math.abs(dur - expected) / expected * 1.6, 0, 1);
          score = a === b ? clamp(0.7 * phoneScore + 0.3 * durScore, 0, 1) : clamp(phoneScore * 0.7, 0, 1);
        } else {
          score = a === b ? 0.8 : 0.45;
        }
        matched++; accSum += score;
        words.push({
          text: a, score, status: score >= .82 ? 'good' : (score >= .6 ? 'mid' : 'bad'),
          refPhon: refPhon || null, saidPhon: saidPhon || null
        });
      } else {
        words.push({ text: T[p.a], score: 0, status: 'bad', refPhon: phonemesOf(T[p.a]), saidPhon: null });
      }
    } else {
      words.push({ text: T[p.a], score: 0, status: 'bad', refPhon: phonemesOf(T[p.a]), saidPhon: null });
    }
  }
  const extras = [];
  for (let bi = 0; bi < saidWords.length; bi++) if (!usedB.has(bi)) extras.push(saidWords[bi]);
  const completeness = T.length ? matched / T.length : 0;
  const accuracy = T.length ? accSum / T.length : 0;
  /* 流利度：词级时间戳 → 真实语速与停顿（严格：语速偏离 ×1.6、停顿惩罚 ×3.2） */
  const wpm = t.duration > 0 ? saidWords.length / (t.duration / 60) : 0;
  const rateScore = clamp(1 - Math.abs(wpm / 150 - 1) * 1.6, 0, 1);
  let pauseMs = 0;
  for (let i = 1; i < t.words.length; i++) {
    const g = t.words[i].start - t.words[i - 1].end;
    if (g > 0.3) pauseMs += g;
  }
  const silRatio = t.duration > 0 ? clamp(pauseMs / (t.duration * 1000), 0, 1) : 0;
  const silScore = clamp(1.2 - silRatio * 3.2, 0, 1);
  const fluency = clamp(0.5 * rateScore + 0.3 * silScore + 0.2 * 1, 0, 1);
  let total = clamp(0.5 * accuracy + 0.3 * fluency + 0.2 * completeness, 0, 1);
  /* 难度系数：句子越难评分越严格 */
  if (level === 3) total = clamp(total * 0.93, 0, 1);
  else if (level === 2) total = clamp(total * 0.97, 0, 1);
  return {
    text: t.text, words, extras, matched,
    accuracy, fluency, completeness, total, level,
    wpm: Math.round(wpm), duration: Math.round(t.duration * 100) / 100,
    targetWords: T, engine: 'whisper-local'
  };
}

/* ---------------- HTTP ---------------- */
function readBody(req, limit = 20 * 1024 * 1024) {
  return new Promise((resolve, reject) => {
    const chunks = []; let size = 0;
    req.on('data', c => {
      size += c.length;
      if (size > limit) { reject(new Error('body too large')); req.destroy(); }
      else chunks.push(c);
    });
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', reject);
  });
}
function json(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, {
    'Content-Type': 'application/json; charset=utf-8',
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Headers': 'Content-Type',
    'Access-Control-Allow-Methods': 'GET,POST,OPTIONS'
  });
  res.end(body);
}
const server = http.createServer(async (req, res) => {
  try {
    if (req.method === 'OPTIONS') { res.writeHead(204, { 'Access-Control-Allow-Origin': '*', 'Access-Control-Allow-Headers': 'Content-Type', 'Access-Control-Allow-Methods': 'GET,POST,OPTIONS' }); res.end(); return; }
    const u = new URL(req.url, 'http://localhost');
    if (u.pathname === '/api/health') {
      json(res, 200, { ok: true, engine: 'whisper-local', model: MODEL, loaded: !!asrPromise });
      return;
    }
    if (u.pathname === '/api/transcribe' && req.method === 'POST') {
      const buf = await readBody(req);
      const t = await transcribeAudio(buf);
      json(res, 200, { ...t, engine: 'whisper-local' });
      return;
    }
    if (u.pathname === '/api/score' && req.method === 'POST') {
      const body = await readBody(req);
      const { audio, target, level } = JSON.parse(body.toString('utf8'));
      if (!audio || !target) { json(res, 400, { error: 'audio and target required' }); return; }
      const buf = Buffer.from(audio, 'base64');
      const t = await transcribeAudio(buf);
      json(res, 200, scoreResult(target, t, Number(level) || 1));
      return;
    }
    /* 静态托管（仅限 ROOT 内） */
    let rel = decodeURIComponent(u.pathname);
    if (rel === '/') rel = '/index.html';
    const full = path.resolve(ROOT, '.' + rel);
    if (!full.startsWith(ROOT) || !existsSync(full)) { res.writeHead(404); res.end('not found'); return; }
    res.writeHead(200, { 'Content-Type': MIME[path.extname(full).toLowerCase()] || 'application/octet-stream' });
    createReadStream(full).pipe(res);
  } catch (e) {
    console.error('[speaklab] error:', e.message);
    json(res, 500, { error: String(e.message || e) });
  }
});
server.listen(PORT, () => {
  console.log(`[speaklab] Whisper engine on http://127.0.0.1:${PORT}`);
  console.log(`[speaklab] model: ${MODEL} · cache: ${env.cacheDir}`);
  console.log('[speaklab] 页面同源访问 /api/*；跨源使用时在前端「设置」里填本地址');
});
