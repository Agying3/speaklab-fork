/* ============================================================
   云游戏卡片 — 前端逻辑
   ------------------------------------------------------------
   配合 server/src/cloud/ 用。后端开一个无头浏览器跑云游戏，
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

   为什么没有云异环：后端 TARGETS 里有 yihuan，但那个入口
   （yh.wanmei.com/cloud/）实测返回 HTTP 514 Frequency Capped，
   它是《异环》官网而不是云游戏入口。做了就是个点了报错的死链，
   所以这里只挂真能用的两个。idl 保留双卡结构，将来异环真出了
   云端入口，加一行 targets 就行。
   ============================================================ */
'use strict';

const CloudCard = (() => {

  /* ---------------- 可调参数 ---------------- */

  const RECONNECT_MS = 1200;       // 断线后多久重连
  const MAX_RECONNECT = 6;         // 连续失败这么多次就停手，改手动点

  /* 页面上有几张卡，以及每张卡对应后端哪个 target。
     顺序就是它们在各自主容器里的排列顺序。
     加新卡片：这里加一项 + index.html 里加一个对应的 .cloud-card。 */
  const CARDS = [
    { el: 'cloudCard',      target: 'genshin'  },
    { el: 'cloudCardStar',  target: 'starrail' },
  ];

  /* ---------------- 单个卡片的实例状态 ----------------
     每张卡各自一条 WebSocket、各自的重连计数、各自的按键集合。
     之前这些是模块级单例，只能撑一张卡；现在收进实例里。 */

  function createCard(el, target){
    const card = {
      el, target,
      img: el.querySelector('.cloud-screen'),
      statusEl: el.querySelector('.cloud-status'),
      ws: null,
      wsUrl: '',
      alive: false,          // 用户是否开过这张卡（关掉就不再重连）
      frames: 0,
      lastFrameAt: 0,
      reconnectTimer: null,
      reconnectCount: 0,
      pressedKeys: new Set(),  // 按下的键，避免 keydown 重复触发
      lastTouchId: 0,
      sleeping: false,       // 页面切到后台了（后端那边已休眠）
    };
    return card;
  }

  const cards = [];

  /* ---------------- 小工具 ---------------- */

  function setStatus(card, text){
    if(!card.statusEl) return;
    card.statusEl.textContent = text || '';
    card.el.classList.toggle('cloud-msg', !!text);
  }

  /* 把浏览器事件换算成 0..1 的归一化坐标。
     用 getBoundingClientRect 拿到卡片在视口里的实际位置，
     再除以宽高。超出 0..1 的一律夹住——后端浏览器收到越界
     坐标会报错，虽然不致命但没必要。 */
  function norm(card, ev){
    const r = card.el.getBoundingClientRect();
    const x = (ev.clientX - r.left) / (r.width || 1);
    const y = (ev.clientY - r.top) / (r.height || 1);
    return { x: Math.min(1, Math.max(0, x)), y: Math.min(1, Math.max(0, y)) };
  }

  function send(card, msg){
    if(card.ws && card.ws.readyState === WebSocket.OPEN){
      try { card.ws.send(JSON.stringify(msg)); } catch(e){}
    }
  }

  /* ---------------- 连接 ---------------- */

  /* 后端地址从 Backend 模块拿，跟其它接口共用一套探测结果。
     把 http:// 换成 ws:// 就行。 */
  function resolveWsUrl(target){
    if(typeof Backend === 'undefined' || !Backend.base) return '';
    const base = Backend.base.replace(/\/+$/, '');
    const wsBase = base.replace(/^http/i, 'ws');
    return `${wsBase}/api/v1/cloud/${target}/ws`;
  }

  function showFrame(card, dataUrl){
    if(!card.img) return;
    card.img.src = dataUrl;
    card.frames++;
    card.lastFrameAt = performance.now();
    card.el.classList.add('cloud-live');
    setStatus(card, '');
  }

  function connect(card){
    if(card.ws) { try { card.ws.close(); } catch(e){} }

    card.ws = new WebSocket(card.wsUrl);

    card.ws.onopen = () => {
      card.reconnectCount = 0;
      setStatus(card, '等待画面…');
    };

    card.ws.onmessage = (ev) => {
      let m;
      try { m = JSON.parse(ev.data); } catch(e){ return; }

      switch(m.type){
        case 'ready':
          // 用真实画面比例撑开卡片，避免黑边或拉伸。
          // 拉伸会让点击坐标对不上，所以用 aspect-ratio 而不是固定高。
          if(m.width && m.height){
            card.el.style.aspectRatio = `${m.width} / ${m.height}`;
          }
          break;
        case 'frame':
          showFrame(card, 'data:image/jpeg;base64,' + m.data);
          break;
        case 'error':
          setStatus(card, m.message || '出错了');
          break;
      }
    };

    card.ws.onclose = () => {
      card.el.classList.remove('cloud-live');
      if(!card.alive) return;            // 用户主动关的，不重连
      // 休眠期间后端可能因为超时把关掉了会话，这条连接就是被它关的。
      // 这时候不要立刻重连——重连等于又起一个浏览器，而用户还在后台，
      // 白白占 700 MB。等他切回来的时候 visibilitychange 会处理。
      if(card.sleeping) return;
      if(card.reconnectCount >= MAX_RECONNECT){
        setStatus(card, '连不上后端，点一下重试');
        return;
      }
      card.reconnectCount++;
      setStatus(card, '重连中…');
      card.reconnectTimer = setTimeout(() => connect(card), RECONNECT_MS);
    };

    card.ws.onerror = () => { /* onclose 会跟着触发，这里不用管 */ };
  }

  function disconnect(card){
    card.alive = false;
    clearTimeout(card.reconnectTimer);
    if(card.ws){
      try { card.ws.onclose = null; card.ws.close(); } catch(e){}
      card.ws = null;
    }
    card.el.classList.remove('cloud-live');
  }

  /* ---------------- 输入注入 ---------------- */

  function bindInput(card){
    const el = card.el;

    /* 指针。用 pointer 事件一套覆盖鼠标/触摸/笔，
       不用分别监听 mouse 和 touch。
       注意 kind 必须是 down/up/move —— 后端只认这三个，
       早先这里发的是 press/release，后端直接报"未知的鼠标类型"
       而且不断开连接，表现就是"点了没反应"。 */
    el.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      el.focus();
      try { el.setPointerCapture(e.pointerId); } catch(err){}
      const p = norm(card, e);
      send(card, { type:'mouse', kind:'down', x:p.x, y:p.y, button:'left' });
    });

    el.addEventListener('pointermove', (e) => {
      // 没按下的移动也要发：云游戏里 hover 会高亮，
      // 而且有些按钮靠 mousemove 才激活。
      const p = norm(card, e);
      send(card, { type:'mouse', kind:'move', x:p.x, y:p.y });
    });

    el.addEventListener('pointerup', (e) => {
      e.preventDefault();
      const p = norm(card, e);
      try { el.releasePointerCapture(e.pointerId); } catch(err){}
      send(card, { type:'mouse', kind:'up', x:p.x, y:p.y, button:'left' });
    });

    // 指针被系统抢走（比如触摸手势）时补一个 up，
    // 否则远端会以为鼠标一直按着。
    el.addEventListener('pointercancel', (e) => {
      const p = norm(card, e);
      send(card, { type:'mouse', kind:'up', x:p.x, y:p.y, button:'left' });
    });

    /* 滚轮 */
    el.addEventListener('wheel', (e) => {
      e.preventDefault();
      const p = norm(card, e);
      send(card, { type:'scroll', x:p.x, y:p.y, dx:e.deltaX, dy:e.deltaY });
    }, { passive:false });

    /* 键盘。卡片获得焦点后才收键，不影响页面上别处的输入。
       云游戏要账号密码，所以字符输入必须支持。 */
    el.addEventListener('keydown', (e) => {
      // 让 Tab 能离开卡片，不然键盘用户被困住
      if(e.key === 'Tab') return;
      e.preventDefault();
      if(card.pressedKeys.has(e.code)) return;   // 系统重复的 keydown 丢掉
      card.pressedKeys.add(e.code);
      send(card, { type:'key', kind:'down', key:e.key, code:e.code, vk:e.keyCode || 0 });
    });

    el.addEventListener('keyup', (e) => {
      if(e.key === 'Tab') return;
      e.preventDefault();
      card.pressedKeys.delete(e.code);
      send(card, { type:'key', kind:'up', key:e.key, code:e.code, vk:e.keyCode || 0 });
    });

    // 卡片里的输入法/粘贴。用 beforeinput 拿不到完整串，直接监听 paste
    el.addEventListener('paste', (e) => {
      const t = (e.clipboardData || window.clipboardData);
      if(!t) return;
      e.preventDefault();
      const text = t.getData('text');
      if(text) send(card, { type:'text', text });
    });

    // 失焦时把按下的键全松开，否则切走再回来会一直"按着"
    el.addEventListener('blur', () => {
      for(const code of card.pressedKeys){
        send(card, { type:'key', kind:'up', key:'', code, vk:0 });
      }
      card.pressedKeys.clear();
    });
  }

  /* ---------------- 对外接口 ---------------- */

  function available(){
    return typeof Backend !== 'undefined' && Backend.ready && !!Backend.caps.cloud_games;
  }

  return {
    /* 卡片能不能用，取决于后端在不在、以及它有没有报告 cloud_games 能力 */
    available,

    init(){
      if(!available()){
        // 后端没这能力就把所有云游戏卡片藏起来，
        // 别让用户点出一个连不上的东西
        for(const spec of CARDS){
          const el = document.getElementById(spec.el);
          if(el) el.hidden = true;
        }
        return;
      }

      for(const spec of CARDS){
        const el = document.getElementById(spec.el);
        if(!el) continue;                  // 页面上没这张卡就跳过
        const card = createCard(el, spec.target);
        cards.push(card);

        card.wsUrl = resolveWsUrl(spec.target);
        if(!card.wsUrl){ el.hidden = true; continue; }
        el.hidden = false;

        // 点一下才连：首页一打开就连会白白启动一个浏览器进程
        const start = () => {
          if(card.alive) return;
          card.alive = true;
          card.reconnectCount = 0;
          setStatus(card, '启动中…');
          connect(card);
        };

        el.addEventListener('click', start);

        // 键盘用户：聚焦后按回车/空格也能启动
        el.addEventListener('keydown', (e) => {
          if(e.key === 'Enter' || e.key === ' '){ e.preventDefault(); start(); }
        }, true);  // 捕获阶段，抢在输入注入的 handler 之前

        bindInput(card);
      }

      // 切到后台就「休眠」，而不是断开。
      //
      // 休眠 = 后端停掉帧流，但浏览器留着、画面留着、登录态留着。
      // 切回来立刻接着玩，不用重新进游戏——重进一次云游戏要几十秒。
      //
      // 说清楚它省什么、不省什么（实测）：
      //   省：帧流（休眠期间一张帧都不发）
      //   不省：内存（约 725 MB 还是占着）、CPU（这页面本来就不烧）
      // 真正把内存还回来的是后端那道休眠超时——默认 5 分钟没人回来
      // 就把会话整个关掉。所以查个攻略无感，去吃饭则会被回收。
      //
      // 一次处理所有卡片：用户切走时，开着的每张卡都该睡。
      document.addEventListener('visibilitychange', () => {
        for(const card of cards){
          if(document.hidden){
            // 还在重连中或是根本没连上，就没什么可睡的
            if(card.ws && card.ws.readyState === WebSocket.OPEN){
              send(card, { type:'sleep' });
              card.sleeping = true;
            }
          } else if(card.sleeping){
            card.sleeping = false;
            // 唤醒前先探一下连接还在不在：切后台期间可能被系统断过、
            // 或者后端已经因为休眠超时把会话关了。
            if(card.ws && card.ws.readyState === WebSocket.OPEN){
              send(card, { type:'wake' });
            } else {
              // 连接没了就重新连，等于重新开会话
              setStatus(card, '重连中…');
              connect(card);
            }
          }
        }
      });
    },

    /* 供控制台/测试用。不带参数时返回第一张卡（保持旧调用可用）。 */
    _debug(target){
      const card = target ? cards.find(c => c.target === target) : cards[0];
      if(!card) return null;
      return {
        target: card.target,
        alive: card.alive,
        frames: card.frames,
        wsUrl: card.wsUrl,
        readyState: card.ws ? card.ws.readyState : -1,
        sinceFrame: card.lastFrameAt ? Math.round(performance.now() - card.lastFrameAt) : -1,
      };
    },
    /* 列出所有卡片，测试用 */
    _all(){ return cards.map(c => ({ target: c.target, frames: c.frames, alive: c.alive })); },
    _stop(){ for(const card of cards) disconnect(card); },
  };
})();

/* 主脚本里调 CloudCard.init()；这里不自动跑，
   因为 Backend.probe() 要先完成才知道后端在不在。 */
