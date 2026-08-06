<h1 align="center">
  <img src="./src-tauri/icons/icon.png" alt="Clash" width="128" />
  <br>
  Continuation of <a href="https://github.com/zzzgydi/clash-verge">Clash Verge</a>
  <br>
</h1>

<h3 align="center">
A Clash Meta GUI based on <a href="https://github.com/tauri-apps/tauri">Tauri</a>.
</h3>

<p align="center">
  Languages:
  <a href="./README.md">简体中文</a> ·
  <a href="./docs/README_en.md">English</a> ·
  <a href="./docs/README_es.md">Español</a> ·
  <a href="./docs/README_ru.md">Русский</a> ·
  <a href="./docs/README_ja.md">日本語</a> ·
  <a href="./docs/README_ko.md">한국어</a> ·
  <a href="./docs/README_fa.md">فارسی</a>
</p>

## TrafficTracer Complete QuickStart

TrafficTracer Complete 将定制 Mihomo 核心、采集/分析 Worker 与 Clash Verge UI 固定在一个仓库中。推荐从 `TrafficTracer` 的 `Complete` 分支进入；捕获、自动分析、会话浏览和代理前五元组查询均可在 UI 完成，不需要手工运行 Python 脚本。

```bash
git clone --branch Complete --recurse-submodules \
  git@github.com:RakuLomis/TrafficTracer.git
cd TrafficTracer
make bootstrap
make check-toolchain
make dev
```

首次运行：

1. 在“订阅/Profiles”导入配置。
2. 在“设置 → Clash 设置 → Clash Core”选择 `verge-mihomo-tt`。
3. 在“代理/Proxies”测速并选择节点。
4. 安装服务，开启 TUN。
5. 打开“流量追踪”，手工填写单目标，或加载 `sites.yaml` 后全选/选择子集；再填写接口、浏览器和输出目录。
6. “检测环境”通过后开始捕获。Capture group 严格按 YAML 顺序逐项执行捕获、Chrome 清理、分析和 checkpoint，最大并发为 1。
7. Capture group 可在 UI 查看进度、取消，并在失败或 Worker 中断后从准确目标继续；每个完成的页面 Session 均可直接打开分析。
8. 捕获运行时“会话”自动打开本次时间戳目录；空闲时默认不显示历史内容，可手动选择当前输出根目录下的时间戳文件夹查看/重新分析。在“规范化流”用代理前五元组查询全部匹配逻辑流和实际代理后五元组。

推荐在每个 `sites[]` 项中填写唯一的 `page_type`（例如 `main-page`、`video-play1`）。输出布局为：

```text
<output-root>/<capture-time>/<domain>/<page_type>__<readable-url>/
├── raw/
└── analysis/pcap/<ordinal>__<readable-url>/
```

PCAP URL 目录中的 `mapping.json` 保留稳定 `connection_id`、全部 URL 和 request ID，因此简化目录不会丢失连接信息。UI 不再暴露独立的 “Serial capture batch” 模块；串行调度只是 Capture group 的内部执行方式。

Sessions 列表只扫描选定 Capture group 的合法 `<domain>/<page>/manifest.json`；不会递归扫描 Session root，也不会把 `.chrome-profiles` 中 Chrome 扩展的 `manifest.json` 误报为损坏会话。旧版直属时间戳 Session 目录仍可手动选择。

当前完整安装包以 Linux x86-64 为验证目标。Complete 页面是 tracing 的唯一 UI 入口，设置侧栏不再有第二个 tracing 开关；TUN 未显式设置 device 时使用 Mihomo 的 `Meta` 默认名。详见 [TrafficTracer Complete UI QuickStart](./docs/TRAFFIC_TRACER_QUICKSTART.md)，其中包含服务授权、`IPC path not ready`、自定义 Session root、批次恢复、采集权限与结果语义。

## Preview

| Dark                             | Light                             |
| -------------------------------- | --------------------------------- |
| ![预览](./docs/preview_dark.png) | ![预览](./docs/preview_light.png) |

## Install

请到发布页面下载对应的安装包：[Release page](https://github.com/clash-verge-rev/clash-verge-rev/releases)<br>
Go to the [Release page](https://github.com/clash-verge-rev/clash-verge-rev/releases) to download the corresponding installation package<br>
Supports Windows (x64/x86), Linux (x64/arm64) and macOS 11+ (intel/apple).

#### 我应当怎样选择发行版

| 版本        | 特征                                     | 链接                                                                                   |
| :---------- | :--------------------------------------- | :------------------------------------------------------------------------------------- |
| Stable      | 正式版，高可靠性，适合日常使用。         | [Release](https://github.com/clash-verge-rev/clash-verge-rev/releases)                 |
| Alpha(废弃) | 测试发布流程。                           | [Alpha](https://github.com/clash-verge-rev/clash-verge-rev/releases/tag/alpha)         |
| AutoBuild   | 滚动更新版，适合测试反馈，可能存在缺陷。 | [AutoBuild](https://github.com/clash-verge-rev/clash-verge-rev/releases/tag/autobuild) |

#### 安装说明和常见问题，请到 [文档页](https://clash-verge-rev.github.io/) 查看

### TG 频道: [@clash_verge_rev](https://t.me/clash_verge_re)

---

## Promotion

### 🤖 [GPTKefu —— 与 Crisp 深度整合的 AI 智能客服平台](https://gptkefu.com)

- 🧠 深度理解完整对话上下文 + 图片识别，自动给出专业、精准的回复，告别机械式客服。
- ♾️ **不限回答数量**，无额度焦虑，区别于其他按条计费的 AI 客服产品。
- 💬 售前咨询、售后服务、复杂问题解答，全场景轻松覆盖，真实用户案例已验证效果。
- ⚡ 3 分钟极速接入，零门槛上手，即刻提升客服效率与客户满意度。
- 🎁 高级套餐免费试用 14 天，先体验后付费：👉 [立即试用](https://gptkefu.com)
- 📢 智能客服TG 频道：[@crisp_ai](https://t.me/crisp_ai)

---

## Features

- 基于性能强劲的 Rust 和 Tauri 2 框架
- 内置[Clash.Meta(mihomo)](https://github.com/MetaCubeX/mihomo)内核，并支持切换 `Alpha` 版本内核。
- 简洁美观的用户界面，支持自定义主题颜色、代理组/托盘图标以及 `CSS Injection`。
- 配置文件管理和增强（Merge 和 Script），配置文件语法提示。
- 系统代理和守卫、`TUN(虚拟网卡)` 模式。
- 可视化节点和规则编辑
- WebDav 配置备份和同步

### FAQ

Refer to [Doc FAQ Page](https://clash-verge-rev.github.io/faq/windows.html)

### Donation

[捐助Clash Verge Rev的开发](https://github.com/sponsors/clash-verge-rev)

## Development

See [CONTRIBUTING.md](./CONTRIBUTING.md) for more details.

To run the development server, execute the following commands after all prerequisites for **Tauri** are installed:

```shell
pnpm i
pnpm run prebuild
pnpm dev
```

TrafficTracer UI users can follow the [TrafficTracer UI Quickstart](./docs/TRAFFIC_TRACER_QUICKSTART.md).

## Contributions

Issue and PR welcome!

## Acknowledgement

Clash Verge rev was based on or inspired by these projects and so on:

- [zzzgydi/clash-verge](https://github.com/zzzgydi/clash-verge): A Clash GUI based on tauri. Supports Windows, macOS and Linux.
- [tauri-apps/tauri](https://github.com/tauri-apps/tauri): Build smaller, faster, and more secure desktop applications with a web frontend.
- [Dreamacro/clash](https://github.com/Dreamacro/clash): A rule-based tunnel in Go.
- [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo): A rule-based tunnel in Go.
- [Fndroid/clash_for_windows_pkg](https://github.com/Fndroid/clash_for_windows_pkg): A Windows/macOS GUI based on Clash.
- [vitejs/vite](https://github.com/vitejs/vite): Next generation frontend tooling. It's fast!

## License

GPL-3.0 License. See [License here](./LICENSE) for details.
