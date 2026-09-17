/* ============================================================
   云游戏卡片 — 前端逻辑
   ------------------------------------------------------------
   配合 server/src/cloud/ 用。后端开一个无头浏览器跑云原神，
   把画面用 Page.startScreencast 编成 JPEG，经 WebSocket 推过来；
   前端只管三件事：

     1. 把 base64 帧画到 <img> 上
     2. 把鼠标/触摸/键盘事件归一化成 0..1 坐标发回去
     3. 断线了自动重连

   为什么坐标要归一化：后端浏览器固定 480x320，而卡片在页面上
   的实际尺寸随窗口变。发归一化坐标，后端乘回自己的尺寸，
   两边都不用知道对方的像素数。

   为什么不自己编码视频：CDP 给的本来就是浏览器编码好的 JPEG，
   后端一次都不重编码，前端也只做一次 decode。这条链路上没有
   任何转码，是这套方案能跑在 1 Mbps 的原因。
   ============================================================ */
'use strict';

const CloudCard = (() => {

  /* ---------------- 可调参数 ---------------- */

  const TARGET = 'genshin';        // 目前只接云原神，另外两个后端已支持
  const RECONNECT_MS = 1200;       // 断线后多久重连
  const MAX_RECONNECT = 6;         // 连续失败这么多次就停手，改手动点
  const POINTER_SCALE = 1;         // 坐标缩放微调，一般不用动

  /* ---------------- 内部状态 ---------------- */

  let el = null;                   // #cloudCard
  let img = null;                  // 显示帧的 <img>
  let statusEl = null;             // 提示文字
  let ws = null;
  let wsUrl = '';
  let alive = false;               // 用户是否开着这张卡（关掉就不再重连）
  let frames = 0;
  let lastFrameAt = 0;
  let reconnectTimer = null;
  let reconnectCount = 0;
  let pressedKeys = new Set();     // 按下的键，避免 keydown 重复触发
  let lastTouchId = 0;
  let sleeping = false;            // 页面是不是切到后台了（后端那边已休眠）

  /* ---------------- 小工具 ---------------- */

  function setStatus(text){
    if(!statusEl) return;
    statusEl.textContent = text || '';
    el.classList.toggle('cloud-msg', !!text);
  }

  /* 把浏览器事件换算成 0..1 的归一化坐标。
     用 getBoundingClientRect 拿到卡片在视口里的实际位置，
     再除以宽高。超出 0..1 的一律夹住——后端浏览器收到越界
     坐标会报错，虽然不致命但没必要。 */
  function norm(ev){
    const r = el.getBoundingClientRect();
    if(!r.width || !r.height) return { x: 0.5, y: 0.5 };
    let x = (ev.clientX - r.left) / r.width;
    let y = (ev.clientY - r.top) / r.height;
    x = Math.min(1, Math.max(0, x)) * POINTER_SCALE;
    y = Math.min(1, Math.max(0, y)) * POINTER_SCALE;
    return { x: +x.toFixed(4), y: +y.toFixed(4) };
  }

  /* 发一个事件给后端。没连上就静默丢掉——
     用户点的时候连接可能正好在重连，报错刷屏没意义。 */
  function send(obj){
    if(!ws || ws.readyState !== WebSocket.OPEN) return false;
    try { ws.send(JSON.stringify(obj)); return true; }
    catch(e){ return false; }
  }

  /* ---------------- 帧渲染 ---------------- */

  /* 直接把 base64 塞给 <img> 的 src，让浏览器自己 decode。
     比手动 createImageBitmap + canvas 省一次拷贝，而且
     浏览器对 data: URL 的图片有解码缓存。

     注意没做「上一帧画完再换下一帧」的背压控制：帧率只有
     4 fps 上下，图片解码远快于这个速度，加了反而多一层延迟。 */
  function drawFrame(m){
    if(!img || !m.data) return;
    img.src = 'data:image/jpeg;base64,' + m.data;
    frames++;
    lastFrameAt = performance.now();
  }

  /* ---------------- WebSocket ---------------- */

  function connect(){
    if(!alive) return;
    clearTimeout(reconnectTimer);

    let ws_;
    try { ws_ = new WebSocket(wsUrl); }
    catch(e){ setStatus('连接失败'); return; }
    ws = ws_;

    ws.onopen = () => {
      reconnectCount = 0;
      setStatus('');
      el.classList.add('cloud-live');
    };

    ws.onmessage = (ev) => {
      let m;
      try { m = JSON.parse(ev.data); } catch(e){ return; }
      switch(m.type){
        case 'ready':
          // 后端告诉实际画面尺寸，按它设卡片比例，避免 letterbox 对不上点击位置
          if(m.width && m.height && el){
            el.style.aspectRatio = m.width + ' / ' + m.height;
          }
          setStatus('');
          break;
        case 'frame':
          drawFrame(m);
          break;
        case 'title':
          // 标题变了说明页面跳转了（比如进了登录页）。不显示文字，
          // 只用来决定要不要清掉"连接中"的提示。
          if(m.title) setStatus('');
          break;
        case 'error':
          setStatus(m.message || '出错了');
          break;
      }
    };

    ws.onclose = () => {
      el.classList.remove('cloud-live');
      if(!alive) return;                 // 用户主动关的，不重连
      // 休眠期间后端可能因为超时把关掉了会话，这条连接就是被它关的。
      // 这时候不要立刻重连——重连等于又起一个浏览器，而用户还在后台，
      // 白白占 700 MB。等他切回来的时候 visibilitychange 会处理。
      if(sleeping) return;
      if(reconnectCount >= MAX_RECONNECT){
        setStatus('连不上后端，点一下重试');
        return;
      }
      reconnectCount++;
      setStatus('重连中…');
      reconnectTimer = setTimeout(connect, RECONNECT_MS);
    };

    ws.onerror = () => { /* onclose 会跟着触发，这里不用管 */ };
  }

  function disconnect(){
    alive = false;
    clearTimeout(reconnectTimer);
    if(ws){
      try { ws.onclose = null; ws.close(); } catch(e){}
      ws = null;
    }
    el.classList.remove('cloud-live');
  }

  /* ---------------- 输入注入 ---------------- */

  function bindInput(){
    /* 鼠标。用 pointer events 一套同时覆盖鼠标、触摸、触控笔，
       比分别监听 mouse* 和 touch* 少一半代码，也不会出现
       同一次点击被两条路径各发一遍的情况。

       注意 kind 的取值是后端定的：down / up / move。
       写 press/release 会被后端拒掉（"未知的鼠标类型"），
       而且只记 debug 日志、不断连接，表现是"点着没反应"，
       很难查——所以这里跟 server/src/cloud/session.rs 的
       InputEvent::Mouse 分支保持一致。 */
    el.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      el.focus({ preventScroll: true });
      const p = norm(e);
      try { el.setPointerCapture(e.pointerId); } catch(err){}
      send({ type:'mouse', kind:'down', x:p.x, y:p.y, button:'left' });
    });

    el.addEventListener('pointermove', (e) => {
      const p = norm(e);
      // 没按下时也发 move：云原神首页有 hover 效果，
      // 而且有些按钮靠 mousemove 才激活。
      send({ type:'mouse', kind:'move', x:p.x, y:p.y });
    });

    el.addEventListener('pointerup', (e) => {
      e.preventDefault();
      const p = norm(e);
      try { el.releasePointerCapture(e.pointerId); } catch(err){}
      send({ type:'mouse', kind:'up', x:p.x, y:p.y, button:'left' });
    });

    // 指针被系统抢走（比如触摸手势）时补一个 up，
    // 否则远端会以为鼠标一直按着。
    el.addEventListener('pointercancel', (e) => {
      const p = norm(e);
      send({ type:'mouse', kind:'up', x:p.x, y:p.y, button:'left' });
    });

    /* 滚轮 */
    el.addEventListener('wheel', (e) => {
      e.preventDefault();
      const p = norm(e);
      send({ type:'scroll', x:p.x, y:p.y, dx:e.deltaX, dy:e.deltaY });
    }, { passive:false });

    /* 键盘。卡片获得焦点后才收键，不影响页面上别处的输入。
       云原神要账号密码，所以字符输入必须支持。 */
    el.addEventListener('keydown', (e) => {
      // 让 Tab 能离开卡片，不然键盘用户被困住
      if(e.key === 'Tab') return;
      e.preventDefault();
      if(pressedKeys.has(e.code)) return;   // 系统重复的 keydown 丢掉
      pressedKeys.add(e.code);
      send({ type:'key', kind:'down', key:e.key, code:e.code, vk:e.keyCode || 0 });
    });

    el.addEventListener('keyup', (e) => {
      if(e.key === 'Tab') return;
      e.preventDefault();
      pressedKeys.delete(e.code);
      send({ type:'key', kind:'up', key:e.key, code:e.code, vk:e.keyCode || 0 });
    });

    // 卡片里的输入法/粘贴。用 beforeinput 拿不到完整串，直接监听 paste
    el.addEventListener('paste', (e) => {
      const t = (e.clipboardData || window.clipboardData);
      if(!t) return;
      e.preventDefault();
      const text = t.getData('text');
      if(text) send({ type:'text', text });
    });

    // 失焦时把按下的键全松开，否则切走再回来会一直"按着"
    el.addEventListener('blur', () => {
      for(const code of pressedKeys){
        send({ type:'key', kind:'up', key:'', code, vk:0 });
      }
      pressedKeys.clear();
    });
  }

  /* ---------------- 对外接口 ---------------- */

  /* 后端地址从 Backend 模块拿，跟其它接口共用一套探测结果。
     把 http:// 换成 ws:// 就行。 */
  function resolveWsUrl(){
    if(typeof Backend === 'undefined' || !Backend.base) return '';
    const base = Backend.base.replace(/\/+$/, '');
    const wsBase = base.replace(/^http/i, 'ws');
    return `${wsBase}/api/v1/cloud/${TARGET}/ws`;
  }

  return {
    /* 卡片能不能用，取决于后端在不在、以及它有没有报告 cloud_games 能力 */
    available(){
      return typeof Backend !== 'undefined' && Backend.ready && !!Backend.caps.cloud_games;
    },

    init(){
      el = document.getElementById('cloudCard');
      if(!el) return;

      // 后端没这能力就整张卡藏起来，别让用户点出一个连不上的东西
      if(!this.available()){
        el.hidden = true;
        return;
      }
      el.hidden = false;

      img = el.querySelector('.cloud-screen');
      statusEl = el.querySelector('.cloud-status');

      wsUrl = resolveWsUrl();
      if(!wsUrl){ el.hidden = true; return; }

      // 点一下才连：首页一打开就连会白白启动一个浏览器进程
      const start = () => {
        if(alive) return;
        alive = true;
        reconnectCount = 0;
        setStatus('启动中…');
        connect();
      };

      el.addEventListener('click', start);

      // 键盘用户：聚焦后按回车/空格也能启动
      el.addEventListener('keydown', (e) => {
        if(e.key === 'Enter' || e.key === ' '){ e.preventDefault(); start(); }
      }, true);  // 捕获阶段，抢在输入注入的 handler 之前

      bindInput();

      // 切到后台就「休眠」，而不是断开。
      //
      // 休眠 = 后端停掉帧流，但浏览器留着、画面留着、登录态留着。
      // 切回来立刻接着玩，不用重新进游戏——重进一次云原神要几十秒。
      //
      // 说清楚它省什么、不省什么（实测）：
      //   省：帧流（休眠期间一张帧都不发）
      //   不省：内存（约 725 MB 还是占着）、CPU（这页面本来就不烧）
      // 真正把内存还回来的是后端那道休眠超时——默认 5 分钟没人回来
      // 就把会话整个关掉。所以查个攻略无感，去吃饭则会被回收。
      document.addEventListener('visibilitychange', () => {
        if(document.hidden){
          // 还在重连中或是根本没连上，就没什么可睡的
          if(ws && ws.readyState === WebSocket.OPEN){
            send({ type:'sleep' });
            sleeping = true;
          }
        } else if(sleeping){
          sleeping = false;
          // 唤醒前先探一下连接还在不在：切后台期间可能被系统断过、
          // 或者后端已经因为休眠超时把会话关了。
          if(ws && ws.readyState === WebSocket.OPEN){
            send({ type:'wake' });
          } else {
            // 连接没了就重新连，等于重新开会话
            setStatus('重连中…');
            connect();
          }
        }
      });
    },

    /* 供控制台/测试用 */
    _debug(){ return { alive, frames, wsUrl, readyState: ws ? ws.readyState : -1,
                       sinceFrame: lastFrameAt ? Math.round(performance.now()-lastFrameAt) : -1 }; },
    _stop(){ disconnect(); },
  };
})();

/* 主脚本里调 CloudCard.init()；这里不自动跑，
   因为 Backend.probe() 要先完成才知道后端在不在。 */
