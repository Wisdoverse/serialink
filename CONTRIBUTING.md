# Contributing to Serialink

## Welcome / 欢迎

Serialink is an open-source tool for working with hardware serial ports in
automation, CI/CD, and AI-assisted workflows. Contributions are welcome in
the form of code, tests, docs, and examples.

This guide is written for external contributors. It explains how to set up
the project, what to verify locally, how to submit a PR, and where
documentation changes belong.

Serialink 是一个面向自动化、CI/CD 和 AI 辅助工作流的开源串口工具。
欢迎提交代码、测试、文档和示例。

本指南面向外部贡献者，说明如何搭建环境、如何在本地验证、如何提交 PR，
以及文档修改应该放在哪里。

## Security / 安全

If you discover a security vulnerability, email `dev@wisdoverse.com`
instead of opening a public issue. We will acknowledge receipt within 48
hours and coordinate a fix before any public disclosure.

If your report includes reproduction steps, affected versions, or a minimal
example, include them. Clear reports are easier to triage and fix.

如果你发现安全漏洞，请直接发送邮件到 `dev@wisdoverse.com`，不要公开
提交 issue。我们会在 48 小时内确认收到，并在公开披露前协调修复。

如果报告里包含复现步骤、受影响版本或最小示例，处理会更快。信息越清晰，
越容易定位和修复问题。

## Development Setup / 开发环境

### Prerequisites / 先决条件

- Rust 1.75+ via [rustup](https://rustup.rs/)
- Linux: `libudev-dev` or `systemd-devel` for the `serialport` crate
- macOS / Windows: no additional system dependencies for the core build path
- `socat` for local testing without physical hardware

```bash
sudo apt install libudev-dev    # Debian/Ubuntu
sudo dnf install systemd-devel  # Fedora
brew install socat              # macOS
```

Rust 1.75+ 可通过 [rustup](https://rustup.rs/) 安装。

Linux 需要 `libudev-dev` 或 `systemd-devel`，因为 `serialport` crate 会用到
相关系统库。macOS 和 Windows 在核心构建路径上不需要额外的系统依赖。

没有真实硬件时，建议安装 `socat` 来创建虚拟串口。Linux 和 macOS 的常见
安装命令如下：

```bash
sudo apt install socat          # Debian/Ubuntu
sudo dnf install socat          # Fedora
brew install socat              # macOS
```

### Clone and Build / 克隆并构建

```bash
git clone https://github.com/Wisdoverse/serialink.git
cd serialink
cargo build
```

Clone the repository, enter the project directory, and build once to verify
your toolchain works.

先克隆仓库，再进入项目目录并执行一次 `cargo build`，确认本地工具链可用。

### Virtual Serial Ports with socat / 使用 socat 创建虚拟串口

```bash
socat -d -d pty,raw,echo=0 pty,raw,echo=0
```

`socat` prints two PTY paths such as `/dev/pts/3` and `/dev/pts/4`. Use one
as the test device and the other to inject or read data.

This is the easiest way to exercise `monitor`, `send`, and the MCP server
without attaching real hardware.

`socat` 会打印两个 PTY 路径，例如 `/dev/pts/3` 和 `/dev/pts/4`。把其中
一个当作被测设备，另一个用于注入或读取数据。

这是在没有真实硬件时测试 `monitor`、`send` 和 MCP 服务的最简单方式。

### Run the CLI / 运行 CLI

```bash
cargo run -- list
cargo run -- monitor /dev/ttyUSB0 -b 115200
cargo run -- send /dev/ttyUSB0 "AT\r\n" -e "OK"
```

Use these commands to confirm port discovery, live monitoring, and
send-and-expect behavior.

使用这些命令可以验证串口发现、实时监控，以及 send-and-expect 的行为。

### Run the MCP Server / 运行 MCP 服务

```bash
cargo run -- serve --mcp
```

This mode reads JSON-RPC messages from stdin and writes responses to stdout.
It is expected to keep running until the parent process exits.

这个模式会从 stdin 读取 JSON-RPC 消息，并向 stdout 输出响应。正常情况
下，它会一直运行，直到父进程退出。

## Local Verification / 本地验证

Run the full local check set before opening a PR:

```bash
cargo test
cargo clippy -- -D warnings
cargo fmt -- --check
```

For focused work, run the smallest command that still proves the change:

```bash
cargo test test_dashboard_page --test http_api_test
cargo test --test http_api_test
```

These commands should stay green before you ask for review. If a change
touches docs only, run at least `cargo fmt -- --check` and the relevant test
slice if the docs describe runtime behavior.

提交 PR 之前，请至少运行以下本地检查：

```bash
cargo test
cargo clippy -- -D warnings
cargo fmt -- --check
```

如果是定向修改，可以运行最小但足够证明行为的测试：

```bash
cargo test test_dashboard_page --test http_api_test
cargo test --test http_api_test
```

这些命令在请求 review 之前应保持通过。若只改文档，至少运行
`cargo fmt -- --check`，并在文档涉及运行时行为时补跑相关测试。

## Workflow And Branching / 工作流与分支

1. Fork the repository on GitHub.
2. Create a branch from `main` with a descriptive name.
3. Keep commits small and focused.
4. Open a PR when the change is ready for review.

Recommended branch prefixes:

| Prefix | Purpose |
|---|---|
| `feat/` | New feature |
| `fix/` | Bug fix |
| `docs/` | Documentation only |
| `refactor/` | Code restructuring |

Commit messages should follow Conventional Commits:

```text
feat: add JSON transform to pipeline engine
fix: prevent panic when serial port disappears during read
docs: update ARCHITECTURE.md with pipeline diagram
refactor: extract reconnect logic into helper function
```

Keep the first line under 72 characters. Add a body only when the change
needs explanation.

1. 在 GitHub 上 fork 仓库。
2. 从 `main` 创建描述清楚的分支。
3. 每个提交保持小而聚焦。
4. 变更准备好后再提交 PR。

推荐的分支前缀如下：

| 前缀 | 用途 |
|---|---|
| `feat/` | 新功能 |
| `fix/` | 修复 bug |
| `docs/` | 仅文档修改 |
| `refactor/` | 代码重构 |

提交信息建议遵循 Conventional Commits：

```text
feat: add JSON transform to pipeline engine
fix: prevent panic when serial port disappears during read
docs: update ARCHITECTURE.md with pipeline diagram
refactor: extract reconnect logic into helper function
```

首行请控制在 72 个字符以内。只有在需要补充说明时才添加正文。

## Pull Request Expectations / PR 期望

Keep one PR to one coherent change. If the change spans behavior, tests, and
docs, make sure they all describe the same user-facing outcome.

Include:

- A short summary of what changed
- The reason the change is needed
- Any commands or tests you ran
- References to related issues when available

If the change affects user-facing behavior, update the relevant docs at the
same time. For example:

- `README.md` for installation, usage, and status
- `ARCHITECTURE.md` for runtime and system design
- `CHANGELOG.md` for released behavior

每个 PR 应该只包含一个完整的变更。如果同时修改了行为、测试和文档，
请确保它们描述的是同一个面向用户的结果。

PR 里请包含：

- 变更摘要
- 变更原因
- 你运行过的命令或测试
- 如有相关 issue，请一并引用

如果变更影响用户可见行为，请同步更新相关文档。例如：

- `README.md` 用于安装、使用和状态说明
- `ARCHITECTURE.md` 用于运行时和系统设计
- `CHANGELOG.md` 用于已发布行为

## Documentation Contributions / 文档贡献

Documentation is a first-class contribution. If you change behavior, update
the docs in the same PR instead of leaving them stale.

Keep the docs aligned:

- `README.md` should describe what users can do now
- `ARCHITECTURE.md` should describe how the system works now
- `CONTRIBUTING.md` should describe how to contribute now
- `CHANGELOG.md` should describe what shipped in each release

When you edit docs:

- Keep English and Chinese sections semantically aligned
- Keep command examples runnable
- Keep status wording explicit
- Prefer short, factual statements over marketing language

If the code, the docs, and the changelog disagree, treat that as a bug and
fix the mismatch in the same change.

文档属于一等贡献。如果你修改了行为，请在同一个 PR 里同步更新文档，
不要让文档继续过时。

请保持文档之间的一致性：

- `README.md` 说明用户现在能做什么
- `ARCHITECTURE.md` 说明系统现在如何工作
- `CONTRIBUTING.md` 说明现在如何贡献
- `CHANGELOG.md` 说明每个版本已经发布了什么

编辑文档时请注意：

- 英文和中文内容要语义对齐
- 命令示例必须能直接运行
- 状态措辞要明确
- 以简短、事实性的表述为主，避免营销化语气

如果代码、文档和 changelog 不一致，请把它当作 bug，并在同一次变更里修复。

## Code Of Conduct / 行为准则

This project follows the
[Contributor Covenant Code of Conduct](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
Please report unacceptable behavior to the maintainers.

本项目遵循
[Contributor Covenant Code of Conduct](https://www.contributor-covenant.org/version/2/1/code_of_conduct/)。
如遇到不当行为，请向维护者报告。

## License / 许可证

By contributing to Serialink, you agree that your contributions will be
licensed under the [Apache License 2.0](LICENSE).

向 Serialink 提交贡献，即表示你同意你的贡献将采用
[Apache License 2.0](LICENSE) 授权。
