docs: 记下仓库来龙去脉（fork 被误删又重建，以及推送为什么要走 API）

为什么要有这个文件
------------------------------------------------------------
这套东西的仓库关系被绕晕过好几次，包括「fork 了但左上角不显示」
和「删错仓库」。把事实记下来，省得下次再从零查一遍。

两个仓库
------------------------------------------------------------
  pengp8029-cmd/speaklab      朋友的原始仓库，只有 2 条提交，不再动
  Agying3/speaklab-fork       我的 fork，所有工作都在这

之前踩的坑：`Agying3/speaklab` 和 `Agying3/speaklab-fork` 曾经同时存在。
09-15 先 fork 出了 `speaklab-fork`，09-16 又另建了一个叫 `speaklab` 的
普通仓库（不是 fork），`origin` 指向它。结果就是提交都推在 `speaklab`
上，而它的 `fork` 字段是 false，GitHub 左上角自然没有 "forked from"。

后来把 `speaklab-fork` 快进到同一内容，`origin` 改指 fork。
再后来手工删仓库时删错了 —— 删掉的是 `speaklab-fork`（真 fork），
留下了 `speaklab`。

好在两边 tree 完全相同（`8de39d5d740a5beea1a3edaf6b385572a2a375e0`），
本地还有 bundle 备份，所以一行代码没丢。重新 fork 了一次
（`POST /repos/pengp8029-cmd/speaklab/forks`），再把 22 个提交整段
补推回新 fork，tree 再次核对一致。

教训：删仓库前先用 API 核 tree 是不是相同——这次核了，所以有惊无险。

推送为什么要走 API
------------------------------------------------------------
这台机器到 github.com:443 时通时不通（`Failed to connect to github.com
port 443`），但 api.github.com 一直可用。`git push` 经常失败。

所以有个兜底脚本 `../api-replay.mjs`（在 H:\toos 下，不在仓库里）：
用 Git Data API 建 blob → tree → commit → 移动 ref，把一段提交链整段
推上去。几个要点：

  1. 文件内容必须按**字节**读（`git show <sha>:<path>` 转 base64），
     绝不能过 `execSync` 的 utf8 字符串——Windows 上会改写非 ASCII。
  2. 空提交（`--allow-empty`，没有改动文件）不能建空 tree，GitHub 报
     422 Invalid tree info，要复用父提交的 tree。
  3. 分支还不存在时 `GET .../git/refs/heads/<b>` 会 404，得先按起点
     提交把 ref 建出来。
  4. 远端走 API 推会重新生成 commit SHA，所以本地跟远端的 SHA 对不上，
     但 tree 一致就说明内容一致。`git fetch` 之后 `reset --hard` 对齐即可。

怎么核对内容有没有丢
------------------------------------------------------------
比对 tree，不要比 commit SHA：

  git rev-parse main^{tree}
  gh api repos/Agying3/speaklab-fork/commits/main --jq .commit.tree.sha

两个值相等 = 文件树逐字节相同。SHA 不同只说明提交对象不同。

备份
------------------------------------------------------------
`H:\toos\speaklab-backup.bundle` 是全分支的 git bundle，删仓库前打的。
需要恢复时：

  git clone speaklab-backup.bundle speaklab-restored
