# TrafficTracer UI Quickstart

This guide starts the TrafficTracer-enabled Clash Verge UI, imports a Mihomo
configuration, selects the TrafficTracer core, and verifies the generated flow
records.

## Current support

- Branch: `feat/traffic-tracer`
- Platform: Linux x86-64
- Core name shown in the UI: `verge-mihomo-tt`
- Default local core source: `../mihomo/bin/mihomo-traffictracer-v2`

Windows, macOS, and other Linux architectures require a matching TrafficTracer
binary and corresponding Tauri bundle configuration.

## 1. Prepare the TrafficTracer core

With `mihomo` and `clash-verge-rev` checked out next to each other:

```shell
cd ../mihomo
./bin/mihomo-traffictracer-v2 -v
```

If that file does not exist, build the TrafficTracer branch and point the UI
prebuild step at the resulting executable. For a broadly compatible x86-64
Linux build:

```shell
cd ../mihomo
make linux-amd64-compatible
```

The prebuild script accepts any explicit core path through
`MIHOMO_TRAFFIC_TRACER_BIN`.

## 2. Install dependencies and prepare sidecars

```shell
cd ../clash-verge-rev
corepack enable
pnpm install
pnpm prebuild
```

When using a core at a non-default location:

```shell
MIHOMO_TRAFFIC_TRACER_BIN=/absolute/path/to/mihomo-traffictracer pnpm prebuild
```

On Linux x86-64, the expected prepared file is:

```text
src-tauri/sidecar/verge-mihomo-tt-x86_64-unknown-linux-gnu
```

## 3. Start the development UI

Close any installed Clash Verge instance first. Two instances should not share
the same service process or controller socket.

```shell
pnpm dev
```

## 4. Configure the UI

1. Open **Profiles** and import a valid Mihomo YAML configuration.
2. Open **Settings → Clash Settings → Clash Core**.
3. Select `verge-mihomo-tt` and allow the core to restart.
4. Open **Proxies**, run the delay test, and select the desired node or group.
5. Enable the system proxy and, when required, TUN mode.
6. Create a writable trace directory:

   ```shell
   mkdir -p /tmp/mihomo-traffictracer
   ```

7. Under **Settings → Clash Settings**, enable **Traffic Tracer**.
8. Set **Traffic Tracer Output** to an absolute path such as:

   ```text
   /tmp/mihomo-traffictracer/trace.jsonl
   ```

The parent directory must already exist and be writable by Clash Verge. An
empty output value sends tracing events to the core's standard output.

## 5. Verify tracing output

Generate traffic through the selected proxy, then inspect the JSON Lines file:

```shell
tail -f /tmp/mihomo-traffictracer/trace.jsonl
```

Given a TCP pre-proxy five-tuple, construct its normalized key and find the
entry event:

```shell
TRACE=/tmp/mihomo-traffictracer/trace.jsonl
FLOW_KEY='tcp|192.168.1.100:54321|1.2.3.4:443'

jq --arg key "$FLOW_KEY" \
  'select(.type == "tcp_connect" and .pre_flow.key == $key)' \
  "$TRACE"
```

Copy the returned `conn_id`, then find the associated post-proxy flow:

```shell
CONN_ID='replace-with-conn-id'

jq --arg id "$CONN_ID" \
  'select(.type == "tcp_proxy_dial" and .conn_id == $id) | .post_flow' \
  "$TRACE"
```

For UDP, find `udp_out` by `pre_flow.key`, then use its `conn_key` to locate
the matching `udp_proxy_dial` event:

```shell
FLOW_KEY='udp|192.168.1.100:54321|8.8.8.8:53'

jq --arg key "$FLOW_KEY" \
  'select(.type == "udp_out" and .pre_flow.key == $key)' \
  "$TRACE"
```

A `post_flow` with `shared: true` represents a reused outer connection and must
not be interpreted as a one-to-one NAT mapping.

## 6. Build a Linux package

Prepare the target-specific core and build normally:

```shell
MIHOMO_TRAFFIC_TRACER_BIN=/absolute/path/to/mihomo-traffictracer \
  pnpm prebuild x86_64-unknown-linux-gnu
pnpm build
```

The Linux Tauri configuration bundles the core as `verge-mihomo-tt` alongside
the standard and alpha cores.

## Troubleshooting

- **No core binaries found**: rerun `pnpm prebuild` and confirm the prepared
  sidecar exists and is executable.
- **`mihomo returned 404 Not Found`**: the standard core is selected. Switch to
  `verge-mihomo-tt` and restart the core.
- **Controller socket connection error**: close duplicate Clash Verge instances
  and restart the selected core.
- **Output path update fails**: use an absolute path and create its parent
  directory before enabling tracing.
- **TUN mode fails**: install or enable the Clash Verge service and confirm that
  the current user has the required system permissions.
