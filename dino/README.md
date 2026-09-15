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
   布局，因此只保留游戏必需的部分，并全部限定在 `#dinoStage` 作用域内。

6. **删掉外链图片引用**。上游 `.icon-offline` 用
   `content: -webkit-image-set(url(assets/...))` 指向仓库内的 PNG，
   内联后这些文件不在，会在控制台报 404，故移除该规则。

7. **键盘事件的取舍**。游戏监听空格 / ↑ / ↓，而本站空格用于播报。
   这里改成只在游戏弹窗打开时转发这三个键，关闭弹窗立刻解绑，
   并且焦点在输入框内时不拦截。

## 重新生成

注入片段由本目录之外的临时脚本生成（`H:\toos\_dino-src\`，未入库）。
如果需要重新做一遍，步骤是：下载上游 → 打上面 1~2 两个补丁 →
base64 内联精灵图与音效 → 替换 CSS → 注入 `index.html`。
