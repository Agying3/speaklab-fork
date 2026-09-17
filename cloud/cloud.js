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
   所以这里不挂。后端保留那一条是给以后留位，等真出了云端入口
   再加一行 CARDS 就行。

   为什么没有云火影忍者：START 里确实有（gameId 700724），但它的
   网页详情页只是宣传页——点「登录后游玩」在浏览器里什么都没发生，
   canonical 的 jump_url 是 `start://start.tencent.com/...`，
   也就是必须装腾讯 START 客户端。纯浏览器起不了串流，做不了。
   ============================================================ */
'use strict';

const CloudCard = (() => {

  /* ---------------- 可调参数 ---------------- */

  const RECONNECT_MS = 1200;       // 断线后多久重连
  const MAX_RECONNECT = 6;         // 连续失败这么多次就停手，改手动点

  /* 页面上有几张卡，以及每张卡对应后端哪个 target。
     顺序就是它们在各自主容器里的排列顺序。
     加新卡片：这里加一项 + index.html 里加一个对应的卡片 div
     （class 带 cloud-poster / cloud-screen / cloud-play / cloud-status）。

     wide = 横卡（跨两列，画面铺满整张卡）。
     没标的（云·鸣潮）是竖卡：只占一列，高度由同行的横卡拉平，
     画面横着居中显示，上下露出海报。两者的差别见下面 ready 那里。 */
  const CARDS = [
    { el: 'cloudCard',      target: 'genshin',   wide: true },
    { el: 'cloudCardStar',  target: 'starrail',  wide: true },
    { el: 'cloudCardMC',    target: 'mingchao'             },
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
      frameW: 0,             // 远端画面尺寸，算点击坐标要用（见 norm）
      frameH: 0,
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
     ------------------------------------------------------------
     这里**不能**直接拿卡片的宽高去除。画面是 object-fit:contain，
     卡片比例跟画面比例不一致时会留黑边，黑边上的点击换算过去
     就偏了（可能差几十像素，云游戏里就是点不中按钮）。

     正确做法是先算出「画面在卡片里实际占的那块矩形」：
     用画面自身的宽高比（frameW/frameH）跟卡片比，
     谁更"扁"就以谁为准，另一边居中留边。
     卡片恰好同比例时（比如 608x405 对 480x320，都是 1.5）
     算出来就等于整个卡片，跟旧行为一致。 */
  function norm(card, ev){
    const r = card.el.getBoundingClientRect();
    const cw = r.width || 1;
    const ch = r.height || 1;

    // 没有画面信息时退回按整张卡片算——总比不响应强
    const fw = card.frameW || 0;
    const fh = card.frameH || 0;
    if(!fw || !fh){
      return {
        x: Math.min(1, Math.max(0, (ev.clientX - r.left) / cw)),
        y: Math.min(1, Math.max(0, (ev.clientY - r.top) / ch)),
      };
    }

    // contain：画面按自身比例缩放，完整放进卡片
    const scale = Math.min(cw / fw, ch / fh);
    const dw = fw * scale;          // 画面实际显示宽
    const dh = fh * scale;          // 画面实际显示高
    const ox = (cw - dw) / 2;       // 左右黑边
    const oy = (ch - dh) / 2;       // 上下黑边

    const x = (ev.clientX - r.left - ox) / (dw || 1);
    const y = (ev.clientY - r.top - oy) / (dh || 1);
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
            card.frameW = m.width;
            card.frameH = m.height;

            // 只有横卡才把画面比例写到卡片上。
            //
            // 竖卡（云·鸣潮）写上去会炸：竖卡靠 align-self:stretch 从
            // 同行那张跨两列的横卡拿到确定的高度（405px），这时候再给
            // 一个 3:2 的 aspect-ratio，浏览器会反过来「由高度算宽度」
            // ——405 * 1.5 = 608px，直接撑破 296px 那一列，卡片横着
            // 压到邻居身上（实测过）。
            //
            // 竖卡不写就没事：没有比例，高度就完全听 stretch 的。
            if(card.wide) card.el.style.aspectRatio = `${m.width} / ${m.height}`;
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

      /* 顺序很重要：**先算坐标，再 focus()**。
         ------------------------------------------------------------
         focus() 默认会把元素滚进视野。首页上云游戏卡排在最下面
         （实测卡顶在 y=1036，视口只有 808 高），一 focus 页面就滚了
         834px。而 norm() 是拿 getBoundingClientRect() 算的——它读到
         的是**滚动之后**的位置，e.clientY 却还是滚动之前的值，两者
         差了一整页，换算出来的 y 永远是 1.0（点到最底下）。

         表现就是：折叠线以下的卡片，第一次点击怎么点都不中。
         在视野内点没事，所以很容易漏掉。

         preventScroll 是第二层保险——就算以后有人把 focus 挪回前面，
         页面也不会滚。老浏览器不认这个参数，所以兜一层。 */
      const p = norm(card, e);
      try { el.focus({ preventScroll: true }); } catch(err){ try { el.focus(); } catch(e2){} }
      try { el.setPointerCapture(e.pointerId); } catch(err){}

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
       云游戏要账号密码，所以字符输入必须支持。

       code/vk 都做了兜底：e.code 在某些输入法或老浏览器下是
       undefined，JSON 化之后变成字段缺失，而后端曾经因为收到
       `code: null` 把整条按键丢掉（日志还是 debug 级，什么都看不见）。
       后端现在两种都收，这里也保证别送 null 过去。 */
    const keyMsg = (kind, e) => ({
      type:'key', kind,
      key: e.key || '',
      code: e.code || '',
      vk: e.keyCode || 0,
    });

    el.addEventListener('keydown', (e) => {
      // 让 Tab 能离开卡片，不然键盘用户被困住
      if(e.key === 'Tab') return;
      e.preventDefault();
      if(card.pressedKeys.has(e.code)) return;   // 系统重复的 keydown 丢掉
      card.pressedKeys.add(e.code);
      send(card, keyMsg('down', e));
    });

    el.addEventListener('keyup', (e) => {
      if(e.key === 'Tab') return;
      e.preventDefault();
      card.pressedKeys.delete(e.code);
      send(card, keyMsg('up', e));
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
        send(card, { type:'key', kind:'up', key:'', code: code || '', vk:0 });
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
        // 远端画面尺寸。点击坐标就是按它换算的，对不上时先看这两个数
        // 跟卡片实际比例差多少（见 norm）。
        frame: card.frameW ? `${card.frameW}x${card.frameH}` : '(还没收到)',
        wide: !!card.wide,
        sleeping: !!card.sleeping,
      };
    },
    /* 列出所有卡片，测试用 */
    _all(){ return cards.map(c => ({ target: c.target, frames: c.frames, alive: c.alive })); },
    _stop(){ for(const card of cards) disconnect(card); },
  };
})();

/* 主脚本里调 CloudCard.init()；这里不自动跑，
   因为 Backend.probe() 要先完成才知道后端在不在。 */
