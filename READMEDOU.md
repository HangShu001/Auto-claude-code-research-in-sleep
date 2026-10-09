# READMEDOU.md — ARIS 落地实操指南（我的实现手册）

> 本仓库 fork 自 [`wanshuiyin/Auto-claude-code-research-in-sleep`](https://github.com/wanshuiyin/Auto-claude-code-research-in-sleep)（ARIS ⚔️ 梦中科研）。
> 本文档是一份**面向我自己**的中文实操手册：从零把 ARIS 跑起来、按我的环境（Windows + Claude Code + DeepSeek 中转 + 本机 RTX5070）完成配置、并说明每条工作流怎么用。
> 所有命令与配置均取自官方 `README.md` 与 `docs/` 目录，关键处标注了原文来源，可追溯。

---

## 一、ARIS 是什么（30 秒版）

ARIS（**A**utonomous **R**esearch via **A**dversarial **M**ulti-Agent **C**ollaboration）是 **84 个可组合的 Claude Code skills**，核心思想是**跨模型对抗协作**：

- **执行者**：Claude Code 读文件、写代码、部署实验、改论文
- **审稿人**：另一家族模型（默认 GPT-6-Astra，走 Codex MCP）以"冷读"方式打分、找弱点、提建议
- **铁律**：执行者与审稿人**必须是不同模型家族**（防止同族盲区）；每轮审稿用**全新 thread**（防止分数滚雪球）；执行者不得审判自己的实验诚实度

七条工作流端到端贯通：**找 idea（W1）→ 实验桥接（W1.5）→ 自动审稿循环（W2）→ 写论文（W3）→ 写 rebuttal（W4）→ 跨 venue 转投（W5）→ 会议演讲（W6）**。实测一晚可将论文从 5/10 推到 7.5/10（含 20+ GPU 实验）。技术报告：[arXiv 2605.03042](https://huggingface.co/papers/2605.03042)。

---

## 二、我需要的环境（前置条件核对表）

| 项目 | 要求 | 我的现状 | 状态 |
|---|---|---|---|
| Git | 任意版本 | Windows 已装 | ✅ |
| Node.js 18+ | `claude` CLI 运行依赖 | 已装（含 Claude Code） | ✅ |
| Python 3.x | 审稿 MCP 桥接、工具脚本 | Anaconda（Py3.12） | ✅ |
| Claude Code CLI | 执行端 | 已装（DeepSeek 中转） | ✅ |
| GPU | 跑实验用（可选） | 本机 RTX5070 8GB | ✅ 够跑小实验 |
| API key | 见下方"模型组合" | DeepSeek key | ✅ |

> 没有 GPU 也能装：Review 和改写类功能不受影响，只有需要跑实验的修复会被跳过（标记"需人工跟进"）。

---

## 三、安装（三条路径，任选其一）

### 路径 A：Claude Code 插件（官方新推，最省事）

```bash
# 1. 添加 marketplace
claude plugin marketplace add wanshuiyin/Auto-claude-code-research-in-sleep

# 2. 安装插件
claude plugin install aris@aris

# 3. 初始化一次（注册内置的 Codex 审稿桥接）
/aris:setup

# 4. 重启 Claude Code
```

安装后 skill 带 `aris:` 前缀（如 `/aris:idea-discovery`）。来源：[README § What's New 2026-09-16](https://github.com/wanshuiyin/Auto-claude-code-research-in-sleep)

### 路径 B：标准安装（git clone + symlink，适合要改 skill 的场景）

```bash
# 1. 把 ARIS 克隆到稳定位置（只需一次）
git clone https://github.com/HangShu001/Auto-claude-code-research-in-sleep.git ~/aris_repo

# 2. 附加到某个论文项目（会在项目里创建 skills 软链）
cd ~/your-paper-project
bash ~/aris_repo/tools/install_aris.sh

# 3. 注册审稿桥接（codex-exec，ARIS 自带，兼容 codex-cli 0.154+）
claude mcp add codex -s user -- python3 "$HOME/aris_repo/mcp-servers/codex-exec/server.py"

# 4. 进入项目使用
claude
```

> Windows 提示：第 2 步的 `bash` 需在 Git Bash 或 WSL 里执行。

### 路径 C：ARIS-Code 独立 CLI（不想依赖 Claude Code 时）

从 [Releases 最新版](https://github.com/wanshuiyin/Auto-claude-code-research-in-sleep/releases/latest) 下载，自带 83 个 skill、内置审稿桥接，无需注册 MCP。详见 [`docs/ARIS-Code-README_CN.md`](docs/ARIS-Code-README_CN.md)。

### ⚠️ codex-cli 0.154 兼容坑（必读）

`codex-cli 0.154.0` 移除了 `codex mcp-server`（旧版审稿入口）。**ARIS 不受影响**：审稿已改为 ARIS 自带的 `mcp-servers/codex-exec/server.py`（驱动 `codex exec`）。**老安装需要重新注册一次**（把上面路径 B 第 3 步执行一遍，先 `claude mcp remove codex -s user`）。来源：[README 顶部 IMPORTANT](https://github.com/wanshuiyin/Auto-claude-code-research-in-sleep)

---

## 四、模型组合：没有 Claude / OpenAI 官方 API 怎么跑（关键）

ARIS 的默认组合是 **Claude（执行）× GPT-6-Astra（审稿）**，需要双官方 API —— 我目前没有。官方内置了 **10 条替代路由（方案 A-I）**，全部不需要 Claude API，其中两条完全不需要任何官方 API。来源：[docs/MODEL_COMBINATIONS_CN.md](docs/MODEL_COMBINATIONS_CN.md)

| 方案 | 执行者 | 审稿人 | 需要 Claude/OpenAI API？ | 适合我？ |
|---|---|---|---|---|
| 默认 ⭐ | Claude Opus/Sonnet | GPT-6-Astra | 都要 | ✗ 无订阅 |
| **方案 A** | GLM-5（Z.ai） | GPT-6-Astra | 只 OpenAI | △ 需 OpenAI |
| **方案 B** | GLM-5（Z.ai） | MiniMax-M3 | 都不要 | ✓ 需两个国产 key |
| **方案 C** | 任意 OpenAI 兼容 | `llm-chat` 接任意 OpenAI 兼容 | 都不要 | ✅ **最推荐（用我现有 key）** |
| 方案 D | Kimi-K2.5 / Qwen3.5+ | GLM-5 / MiniMax-M3 | 都不要 | ✓ 阿里百炼一个 key 双端 |
| **方案 E 🆓** | DeepSeek-V3.1 / Qwen3-Coder | DeepSeek-R1 / Qwen3-235B | 都不要 | ✅ 免费（ModelScope 2000 次/天） |
| 方案 C.1 | 任意 | OpenRouter 固定模型 | 都不要 | ✓ |

### 我的推荐配置

**方案 C（首选，直接用我现有 DeepSeek 中转）：**

我的 Claude Code 执行端已指向 DeepSeek-V4-flash（api.deepseek.com/anthropic）。按方案 C，再配一个 `llm-chat` MCP 作为审稿人，指向**另一家族**的 OpenAI 兼容 API（例如 Z.ai 的 GLM 或阿里百炼的 MiniMax/Qwen）——满足"跨族审稿"铁律：

```bash
# 配置 llm-chat 审稿 MCP（以 Z.ai GLM 为例）
claude mcp add llm-chat -s user -- python3 "$HOME/aris_repo/mcp-servers/llm-chat/server.py" \
  --api-key YOUR_ZAI_KEY \
  --base-url https://api.z.ai/api/anthropic \
  --model glm-5
```

> 完整混搭指南：[docs/LLM_API_MIX_MATCH_GUIDE.md](docs/LLM_API_MIX_MATCH_GUIDE.md)

**方案 E（零成本备选，ModelScope 免费）：**

```bash
# 按 docs/MODELSCOPE_GUIDE.md 注册 ModelScope key（免费 2000 次/天）
# 执行者 DeepSeek-V3.1 × 审稿人 DeepSeek-R1
```

> ⚠️ 注意：同一提供商的不同模型（如 DeepSeek-V4-flash × DeepSeek-R1）在严格意义上不算"跨家族"，官方示例的跨族组合是 GLM×DeepSeek、Kimi×MiniMax 这类。最优解仍是**执行端 DeepSeek + 审稿端 GLM/Qwen**。

**配置完成后验证**：让模型读一遍项目并试跑 skill：

```
读一下这个项目，验证所有 skills 是否正常：
/idea-creator, /research-review, /auto-review-loop, /novelty-check,
/idea-discovery, /research-pipeline, /research-lit, /run-experiment
```

---

## 五、快速开始

### 基本模式：给一个研究方向，全自动处理

```bash
/research-pipeline "factorized gap in discrete diffusion LMs"
```

### 目标模式：已有论文 + 代码，针对性改进

```bash
/research-pipeline "improve method X" --ref-paper https://arxiv.org/abs/xxxx --base-repo https://github.com/org/project
```

### 单条工作流独立使用

| 场景 | 命令 |
|---|---|
| 只有一个研究方向，找该做什么 | `/idea-discovery "研究方向" --effort max` |
| 已有实验计划，让它跑起来 | `/experiment-bridge --base repo: https://github.com/...` |
| 明早要交，让它通宵审改 | `/auto-review-loop "focus on Section 3-5, our CRF results are weak" --difficulty nightmare` |
| 已有结果，写成可投稿 PDF | `/paper-writing NARRATIVE_REPORT.md --venue: ICLR --effort max` |
| 收到审稿意见，起草 rebuttal | `/rebuttal "paper/ + reviews" --venue ICML --character limit 5000` |
| 中稿后做会议 PPT | `/paper-talk "paper/" --venue ICLR` |

> 常用参数：`--effort lite/max/beast`（投入度）、`--assurance draft/polished/conference-ready`（保证等级）、`--MAX_ROUNDS 4`（审稿循环上限，默认 4）。

---

## 六、GPU 配置（本机 RTX5070 直接访问）

在**论文项目根目录的 `CLAUDE.md`** 里加这段（我本机有 GPU，用"直接访问"模板；官方模板见 [docs/GPU_SETUP_CN.md](docs/GPU_SETUP_CN.md)）：

```markdown
## GPU 环境
- 这台机器有直接 GPU 访问（不需要 SSH）
- GPU：RTX 5070 8GB
- 实验环境：`deep_learning`（Python 3.12 + PyTorch，Anaconda）
- 激活命令：`conda activate deep_learning`
- 代码目录：`D:\1_study\projects\experiments\`
- 后台运行用 `screen`（WSL）或 `start /b`（Windows）
```

**没有 GPU / 显存不够？** 官方提供 Vast.ai 按需租用（ARIS 自动挑最便宜的 GPU，按总成本排序，跑完自动销毁，`max_budget` 可设上限）：

```markdown
## Vast.ai
- gpu: vast
- auto_destroy: true
- max_budget: 5.00
```

典型花费：RTX 4090 消融实验约 $0.30–2/次，A100/H100 baseline 约 $2–10/次。详见 [docs/integrations/VAST_GPU_GUIDE_CN.md](docs/integrations/VAST_GPU_GUIDE_CN.md)。

> 审稿人（GPT/GLM 等）**只决定做什么实验**，Claude Code 根据我的 `CLAUDE.md` 负责怎么跑。

---

## 七、项目文件结构（ARIS 输出规范）

ARIS 工作流在项目目录产出分层文件（来源：[docs/PROJECT_FILES_GUIDE_CN.md](docs/PROJECT_FILES_GUIDE_CN.md)）：

```
project/
├── CLAUDE.md                     # 仪表盘：Pipeline 状态 + 项目约束
├── findings.md                   # 轻量发现日志（实验异常/根因/决策，随时追加）
├── MANIFEST.md                   # 产出追踪清单（自动维护）
├── idea-stage/                   # W1 找 idea 的产出
│   ├── IDEA_REPORT.md            #    brainstorm 原始 8-12 个 idea + pilot 结果
│   ├── IDEA_CANDIDATES.md        #    评审后存活的候选池（3-5 个）
│   └── docs/research_contract.md #    当前 idea 的聚焦上下文
├── refine-logs/                  # W1.5 实验规划与精炼
│   ├── EXPERIMENT_PLAN.md        #    实验设计（claim + blocks + 算力预算）
│   ├── EXPERIMENT_TRACKER.md     #    执行清单（TODO → DONE）
│   ├── EXPERIMENT_RESULTS.md     #    收集的结果
│   ├── EXPERIMENT_LOG.md         #    完整实验记录（成功失败都记）
│   └── FINAL_PROPOSAL.md         #    最终精炼提案
├── review-stage/                 # W2 自动审稿产出
│   ├── AUTO_REVIEW.md            #    审稿循环日志（评分/行动）
│   └── REVIEW_STATE.json         #    恢复状态（上下文压缩后续跑）
├── paper/                        # W3 论文产出（main.tex + roundN/ 每轮 PDF）
└── research-wiki/                # 持久化知识库（papers/ ideas/ experiments/ claims/）
```

**会话恢复顺序**（新会话/压缩后按序读）：`CLAUDE.md` → `research_contract.md` → `findings.md` 最近条目 → `EXPERIMENT_LOG.md`（按需）。

---

## 八、避坑与成本提示

1. **审稿人独立性是红线**：每轮 review 必须开新 thread，禁止用 `codex-reply` 续聊旧 review（实测会从 3/10 滚雪球到 8/10）
2. **跨族是硬要求**：执行者 ≠ 审稿者家族，同族审稿是"非特性"，宁可用免费 DeepSeek/Gemini 凑一个
3. **审稿循环有护栏**：`MAX_ROUNDS=4` 防死循环；>4 GPU 小时的实验自动跳过并标记人工跟进；禁止隐藏弱点刷分
4. **成本控制**：找 idea 的 pilot 单卡 1-2 小时就够，别上大卡；Vast.ai 设 `max_budget`；模型侧用 DeepSeek/GLM 这类低价 key，审稿量大的夜间循环优先低价模型
5. **升级提醒**：官方更新会写在 README "What's New"；skill 更新执行 `bash tools/smart_update.sh --apply`
6. **输出双版本**：ARIS 产出带时间戳文件（历史）+ 固定名文件（最新），`MANIFEST.md` 是中央索引，别手删

---

## 九、官方文档索引（docs/ 目录）

| 文档 | 用途 |
|---|---|
| [ARIS_INTRO.md](docs/ARIS_INTRO.md) | 完整介绍：架构、7 工作流、84 skills |
| [SKILLS_CATALOG.md](docs/SKILLS_CATALOG.md) | 84 个 skill 全目录 |
| [MODEL_COMBINATIONS_CN.md](docs/MODEL_COMBINATIONS_CN.md) | 10 条替代模型路由（我的核心参考） |
| [GPU_SETUP_CN.md](docs/GPU_SETUP_CN.md) | GPU 服务器配置（SSH/本机/Vast.ai） |
| [PROJECT_FILES_GUIDE_CN.md](docs/PROJECT_FILES_GUIDE_CN.md) | 项目文件结构与产出规范 |
| [SESSION_RECOVERY_GUIDE_CN.md](docs/SESSION_RECOVERY_GUIDE_CN.md) | 断点恢复 |
| [CUSTOMIZATION_CN.md](docs/CUSTOMIZATION_CN.md) | 按 skill 定制 |
| [MODELSCOPE_GUIDE.md](docs/MODELSCOPE_GUIDE.md) | ModelScope 免费方案 E |
| [MINIMAX_MCP_GUIDE.md](docs/MINIMAX_MCP_GUIDE.md) | MiniMax 方案 B |
| [LLM_API_MIX_MATCH_GUIDE.md](docs/LLM_API_MIX_MATCH_GUIDE.md) | llm-chat 自由混搭方案 C |

---

*本文档由我的 AI 助手基于官方 README 与 docs 整理生成，安装命令请以官方最新文档为准（上游更新频繁）。*