# TrafficTracer Complete UI QuickStart

本指南面向 TrafficTracer Complete 最小完整版本。代理配置、核心/节点选择、TUN、捕获、自动分析、会话管理和五元组查询均由 Clash Verge UI 完成；正常使用不需要手工运行 `capture.py`、`analyze.py` 或 `query_flow.py`。

## 支持范围与依赖

当前验证平台为 Linux x86-64。需要可用的 Mihomo YAML、Chrome/Chromium、`tshark`、`dumpcap` 和可写的绝对输出目录。完整双接口捕获还需要 Clash Verge 服务与 TUN。

Debian/Ubuntu 可执行：

```bash
sudo apt-get install tshark
sudo setcap cap_net_raw,cap_net_admin=eip "$(command -v dumpcap)"
dumpcap -D
```

使用 `wireshark` 用户组的发行版需按软件包提示添加当前用户并重新登录。

## 1. 安装或开发运行

安装 Complete 构建的 Deb：

```bash
sudo apt install ./Clash\ Verge_2.5.2_amd64.deb
```

或启动 AppImage：

```bash
chmod +x ./Clash\ Verge_2.5.2_amd64.AppImage
./Clash\ Verge_2.5.2_amd64.AppImage
```

文件名随版本变化。Complete 包同时携带 `verge-mihomo-tt`、TrafficTracer Worker 和服务 helper，不能用上游 Clash Verge 包替代。

开发运行：

```bash
git clone --branch Complete --recurse-submodules \
  git@github.com:RakuLomis/TrafficTracer.git
cd TrafficTracer
make bootstrap
python -m pip install -r requirements.txt -r requirements-build.txt
corepack enable
cd components/clash-verge-rev
pnpm install --frozen-lockfile
cd ../..
make check-toolchain
make dev
```

`make dev` 会构建并注入固定核心与 Worker。只编译和准备 sidecar、不启动第二个 UI 时运行 `make prepare-dev`。不要让开发版与已安装的 Clash Verge 同时运行并争用 socket；当前实例正在提供网络时，先构建，等到维护窗口再从旧 UI 正常退出后启动开发版，不要强制 kill。

## 2. 配置代理

1. 在“订阅/Profiles”导入并激活 YAML。
2. 在“设置 → Clash 设置 → Clash Core”选择 `verge-mihomo-tt` 并等待重启。
3. 在“代理/Proxies”测速并选择节点或策略组。
4. 按需开启系统代理。
5. 完整捕获时安装 Clash Verge 服务并开启 TUN。

标准 `verge-mihomo` 没有 tracing 能力。诊断报告能力缺失或 404 时，重新选择 `verge-mihomo-tt`。

## 3. 服务授权、IPC 与 TUN

在设置页点击安装服务。Linux 授权窗口会执行与主程序同目录的 helper，例如：

```text
/usr/bin/sh -c /usr/bin/clash-verge-service-install
```

开发版 helper 位于 Tauri 运行目录。确认路径属于当前程序后授权。安装成功后开启 TUN，并重新检测环境。

| 路径 | 用途 |
| --- | --- |
| `/run/clash-verge-service/service.sock` | Linux 特权服务 IPC（service v2.6.1） |
| `/tmp/verge/verge-mihomo.sock` | Mihomo 控制器 IPC |

### `IPC path not ready`

UI 安装后最多等待约 15 秒完成 socket 与协议握手。Complete 固定使用 service/client v2.6.1；若旧包仍提示 `install Service failed: IPC path not ready`，先确认它是否仍在错误地检查 `/tmp/verge/clash-verge-service.sock`。

不要为了排障停止当前正在提供网络的 Clash Verge。先在另一个终端只读检查 helper、进程、systemd 状态和实际 socket：

```bash
ls -l /usr/bin/clash-verge-service*
pgrep -af 'clash-verge-service|clash-verge'
systemctl status clash-verge-service --no-pager
ls -l /run/clash-verge-service/service.sock
```

如果 `/run/clash-verge-service/service.sock` 已存在且服务为 active，而 UI 仍报告旧 `/tmp/verge/...` 路径，说明运行的是旧 UI 客户端，需要安装同一 Complete 构建中的 UI 与 service；不要反复重装健康的服务。只有 socket 不存在或协议检查明确报告不兼容时，才在有可接受的网络维护窗口后使用 UI 的“修复/重新安装服务”。

不要同时手工启动 helper 和点击 UI 安装，也不要删除正在使用的 socket。

## 4. 捕获与环境诊断

打开“流量追踪”后有两种目标来源：

- “手工输入”：填写目标 URL/域名、1–86400 秒持续时间和 TCP/UDP/全部；
- “YAML 配置”：浏览并加载预先编写的 `.yaml`/`.yml`，再从 `sites` 列表选择一个目标。URL、域名、持续时间、协议、页面加载超时和运行标签由该项提供。

目标配置兼容独立 TrafficTracer 的格式，例如：

```yaml
sites:
  - domain: example.com
    url: https://example.com/
    wait: 15
    traffic_type: all
    wait_load_timeout: 30
```

只读取 `sites`；即使同一文件含有 `global.mihomo`、`global.chrome`、`global.network` 或 `global.output`，UI 也不会应用它们，代理与运行环境仍由 Clash Verge 管理。`wait` 是捕获持续时间，`wait_load_timeout` 是页面加载超时；`traffic_type` 的 `tcp`、`udp`、`all` 直接映射协议，其他安全标签会将协议回退为 `all` 并显示警告。加载结果不会包含代理 secret。

加载后会记录文件绝对路径、SHA-256 和目标序号。每次开始捕获前后端都会重新加载并逐字段核对；如果文件在预览后被修改，捕获会拒绝启动，点击刷新并重新选择即可。当前每次捕获一个选中目标，不会自动批量运行整个 `sites` 列表。

目标 YAML 应放在不会随重启清理的持久目录，不要使用 `/tmp`。文件必须为 UTF-8、扩展名 `.yaml`/`.yml`、大小不超过 1 MiB，并包含非空 `sites`。`domain` 必须是有效 DNS 名，`url` 必须是绝对 HTTP(S) URL；`wait` 范围 1–86400，`wait_load_timeout` 范围 1–3600；`traffic_type` 最多 64 位，只能使用字母、数字、点、下划线和连字符，首位必须为字母或数字。

随后填写 TUN 接口、物理接口、Chrome 绝对路径和会话输出绝对目录。可选择数据包、CDP、NetLog、自动分析和无头模式。

`ip route show default` 可确认物理出口，`ip -brief link` 可查看 TUN 接口。

点击“检测环境”。诊断覆盖核心能力、控制器、TUN 服务、接口、捕获权限、浏览器、输出目录和磁盘。“已阻断”必须修复；“需要处理”表示可能降级。修改字段后需重新检测。

通过后点击“开始捕获”。运行期间核心、配置、tracing、TUN、系统代理和服务操作会锁定。关闭页面不会取消任务；停止时使用“取消任务”。启用“自动分析”后 Worker 会直接分析，无需 Python 终端。

## 5. 会话与产物

“会话”显示状态、组件版本、警告和产物。可打开详情/目录，查看 trace、CDP、NetLog、pcap 和关联结果，或“重新分析”。

输出根目录下每个子目录是一条 session；manifest 是 UI 与 Worker 识别状态、产物的事实来源。任务运行时不要移动或修改它。

## 6. 五元组查询

在“规范化流 → 查询代理前五元组”填写协议、源 IP/端口、目的 IP/端口，点击“查询全部会话”。UI 返回全部匹配逻辑流，并展示代理前/后五元组、匹配状态、置信度、连接/外层连接 ID 和浏览器请求上下文。

- `matched`：明确关联；
- `ambiguous`：存在多个合理候选；
- `unmatched`：证据不足；
- “共享”表示多个逻辑流复用外层连接，不是一对一 NAT 映射；
- 代理后五元组为空表示未观测到完整拨号结果，UI 不会复制或猜测代理前五元组。

一个代理前五元组可能返回零条、一条或多条逻辑流；`post_flow` 只陈述实际观测结果。

## 7. 常见故障

- **核心能力缺失/404**：选择 `verge-mihomo-tt`，重启并重新检测。
- **Worker unavailable/API mismatch**：重装 Complete 包；开发模式运行 `make prepare-dev`。
- **找不到 tshark/dumpcap**：安装 Wireshark CLI 并确认应用可从 `PATH` 找到。
- **Capture permission denied**：用 `dumpcap -D` 验证，修复 capabilities/用户组后重新登录。
- **找不到接口**：先成功开启 TUN 并刷新；网络切换后重新检测物理出口。
- **Chrome 不可执行**：选择 Chrome/Chromium 绝对路径。
- **输出不可写/空间不足**：选择可写绝对目录并释放空间。
- **没有 post_flow**：检查状态、错误、匹配原因和 shared；失败、取消或复用连接不保证独占代理后五元组。
- **结果不完整**：确认 TUN、两个接口和数据包/CDP/NetLog 选项后重试。

## 8. Linux 打包与验收

从 TrafficTracer `Complete` 根目录运行统一流水线，不要在 UI 子仓库直接执行裸 `pnpm tauri build`，否则可能打入陈旧的核心或 Worker：

```bash
cd /absolute/path/to/TrafficTracer
make check-toolchain
make test-python
make test-contracts
make prepare-dev
TT_PACKAGE_OUTPUT_DIR="$PWD/dist/packages/target-config-v2" make package-linux
sha256sum -c dist/packages/target-config-v2/SHA256SUMS
```

这里的 `$PWD` 必须是 TrafficTracer `Complete` 仓库根目录；不要从 `~` 直接复制执行，否则路径会展开到 `$HOME/dist/...`。

`make package-linux` 会再次执行 sidecar 准备，因此单独的 `make prepare-dev` 是便于提前验证，不是打包必需步骤。默认输出目录已存在时构建会拒绝覆盖；为每个候选指定新的 `TT_PACKAGE_OUTPUT_DIR`。启用 updater 时还需 `TAURI_SIGNING_PRIVATE_KEY`；没有私钥时统一流水线会关闭 updater artifact 并生成经过布局验证的 unsigned 包。

统一流水线已经验证 Deb/AppImage 都携带 7 个可执行文件。如需手工复核：

```bash
cd components/clash-verge-rev
pnpm verify:linux-bundle -- \
  --target x86_64-unknown-linux-gnu \
  --sidecars src-tauri/sidecar \
  /absolute/path/to/package.deb \
  /absolute/path/to/package.AppImage
cd ../..
```

Worker API v2 必须与新版 UI 成套部署。安装 Deb 不会更新已经运行的旧进程；先构建并校验，到可接受的网络维护窗口后从旧 UI 正常退出，再安装新包并启动：

```bash
cd /absolute/path/to/TrafficTracer
sudo apt install "$PWD/dist/packages/target-config-v2/Clash Verge_2.5.2_amd64.deb"
```

不要从会被重启清理的 `/tmp` 安装。升级后重新确认 `verge-mihomo-tt`、TUN 服务和环境诊断。

干净安装应验证：导入配置；TT 核心；测速选节点；系统代理/TUN；环境无阻断；UI 捕获/自动分析；会话产物；已知代理前五元组的全部逻辑流与实际 `post_flow` 状态。
