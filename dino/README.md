# dino/ — T-Rex Runner

首页「休息一下」卡片里的小游戏。代码不是本项目原创。

## 来源

- 上游：<https://github.com/wayou/t-rex-runner>
- 原始出处：Chromium 的离线小恐龙（`components/neterror/resources/`）
- 许可证：**BSD 3-Clause**，全文见同目录 `LICENSE`
- 版权：`Copyright (c) 2014 The Chromium Authors` / `Copyright (c) 2022, 牛さん`

BSD-3 要求保留版权声明与许可全文，因此 `LICENSE` 必须随代码一起分发，
`index.html` 里注入的那段 JS 也保留了文件头的版权注释。

## 注入方式

游戏代码内联在 `index.html` 里，不是一个独立文件，原因是本项目坚持
单文件、无外部依赖、无 CDN、无构建步骤。精灵图与音效都以 base64
内联，不产生任何网络请求。

## 相对上游改了什么

1. **去掉自动启动**。上游在 `DOMContentLoaded` 时执行
   `new Runner('.interstitial-wrapper')`，会让每次打开页面都初始化
   canvas 和 AudioContext。本站改成点击卡片时才创建。

2. **导出构造函数**。原代码是 IIFE，`Runner` 不对外可见，末尾加了
   `window.TRexRunner = Runner` 以便按需实例化。

3. **重置单例**。`Runner` 内部用 `Runner.instance_` 做单例，已存在就
   直接返回旧实例；而本站的弹窗每次关闭都会销毁 DOM，因此关闭时把
   `instance_` 一并清空，否则第二次打开会拿到指向已删除 DOM 的实例。

4. **补隐藏占位元素**。上游 `init()` 直接
   `document.querySelector('.icon-offline').style...`，取不到就抛
   `TypeError`。本站没有离线图标，故在舞台里放一个隐藏的
   `.icon / .icon-offline` 占位元素。

5. **剔除会污染页面的 CSS**。上游 `index.css` 含
   `html, body { height:100%; margin:0 }`、全局 `h1` 等规则，会破坏本站
   布局，因此只保留游戏必需的部分，并全部限定在 `#dinoCard` 作用域内。

6. **删掉外链图片引用**。上游 `.icon-offline` 用
   `content: -webkit-image-set(url(assets/...))` 指向仓库内的 PNG，
   内联后这些文件不在，会在控制台报 404，故移除该规则。

7. **键盘事件的取舍**。游戏监听空格 / ↑ / ↓，而本站空格用于播报。
   这里改成只在卡片被点开、操作权交给用户后才转发这三个键，
   交还控制权时立刻解绑，并且焦点在输入框内时不拦截。

8. **合成按键要有 target**。上游 `onKeyDown` 的第一层判断是
   `e.target != this.detailsButton`，而本站没有 `detailsButton`
   （值为 `null`）；如果合成事件也传 `target: null`，条件为假，
   整段按键逻辑会被静默跳过。因此托管控件自动跳时显式传入 canvas
   作为 target。

9. **停用「街机模式」的缩放**。上游 `startGame()` 会调
   `setArcadeMode()`，其中 `setArcadeModeContainerScale()` 按窗口算出
   `max(1, innerHeight / 150)` 并给容器加 `transform: scale(...)`；
   在桌面上那就是 4~6 倍，是给 Chromium 离线页整页玩游戏用的，
   放进卡片会把画面直接顶出去。这里覆写掉这两个方法，并在 CSS 上
   用 `transform: none !important` 兜一层，防止它在 `fit()` 之后又被写回。

10. **音效缺失时静音继续**。上游 `loadSounds()` 取 template 和音频
    元素时都不判空，一旦取不到就在 `.src` 上抛 `TypeError`；而这个
    异常发生在 `onKeyDown` 内部，会连带把按键逻辑一起中断，表现为
    恐龙再也跳不动。这里都加了判空，取不到就不加载音效。

11. **画面按容器等比缩放**。上游画布高度固定 150，宽度跟着容器走，
    而卡片里的游戏区只有约 106~124px，直接放会裁掉地面。这里以游戏
    画布的原始尺寸为基准做 `transform: scale()`，画面完整放进游戏区，
    恐龙与地面一起缩小、比例不变。缩放基准只在首次记录一次，避免
    反复计算时受 `updateCanvasScaling` 改写 `canvas.width` 的影响。

## 呈现方式

游戏嵌在首页卡片网格的第四张卡片里，**不弹窗、不跳路由、不改变卡片
尺寸**（与其它三张 `.entry` 卡片等高）：

- **未点击**：托管控件替你玩——盯着 `horizon.obstacles[0]` 提前起跳，
  撞了就重开，所以画面一直在动，本身就说明「这里可以玩」。
- **点击后**：提示文字淡出，键盘交给用户。再点一次即交还，回到托管。
- 卡片滚出视口或标签页切到后台时停掉游戏，避免白白占着 rAF 和音频。

## 重新生成

注入片段由本目录之外的临时脚本生成（`H:\toos\_dino-src\`，未入库）。
如果需要重新做一遍，步骤是：下载上游 → 打上面 1~2 两个补丁 →
base64 内联精灵图与音效 → 替换 CSS → 注入 `index.html`。
