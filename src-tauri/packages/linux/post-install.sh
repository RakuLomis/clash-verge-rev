#!/bin/bash
required_binaries="
clash-verge-service
clash-verge-service-install
clash-verge-service-uninstall
verge-mihomo
verge-mihomo-alpha
verge-mihomo-tt
traffictracer-worker
"

for binary in $required_binaries; do
    binary_path="/usr/bin/$binary"
    if [ ! -f "$binary_path" ]; then
        echo "TrafficTracer Complete installation is missing $binary_path" >&2
        exit 1
    fi
    chmod 0755 "$binary_path"
done

. /etc/os-release

if [ "$ID" = "deepin" ]; then
    PACKAGE_NAME="$DPKG_MAINTSCRIPT_PACKAGE"
    DESKTOP_FILES=$(dpkg -L "$PACKAGE_NAME" 2>/dev/null | grep "\.desktop$")
    echo "$DESKTOP_FILES" | while IFS= read -r f; do
        if [ "$(basename "$f")" == "Clash Verge.desktop" ]; then
            echo "Fixing deepin desktop file"
            mv -vf "$f" "/usr/share/applications/clash-verge.desktop"
        fi
    done
fi
