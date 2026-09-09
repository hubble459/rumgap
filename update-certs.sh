#!/usr/bin/env bash
# Copies a cert+key pair exported from Zoraxy (a "<hostname>_export.zip"
# containing "<hostname>.pem" + "<hostname>.key") onto the rumgap server and
# reloads nginx to pick them up.
#
# Usage: ./update-certs.sh ~/Downloads/api.manga.quentincorreia.nl_export.zip

set -euo pipefail

SERVER_HOST="192.168.1.54"
SERVER_USER="quentin"
REMOTE_CERTS_DIR="/opt/rumgap/certs"

if [ $# -ne 1 ]; then
	echo "Usage: $0 <export.zip>" >&2
	exit 1
fi

ZIP_PATH="$1"
if [ ! -f "$ZIP_PATH" ]; then
	echo "File not found: $ZIP_PATH" >&2
	exit 1
fi

echo "Checking connectivity to $SERVER_HOST..."
if ! ping -c 1 -W 2 "$SERVER_HOST" > /dev/null 2>&1; then
	echo "Can't reach $SERVER_HOST - are you connected to the VPN/LAN?" >&2
	exit 1
fi

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

unzip -o "$ZIP_PATH" -d "$WORKDIR" > /dev/null

PEM_FILE=$(find "$WORKDIR" -name '*.pem' | head -n1)
KEY_FILE=$(find "$WORKDIR" -name '*.key' | head -n1)

if [ -z "$PEM_FILE" ] || [ -z "$KEY_FILE" ]; then
	echo "Couldn't find a .pem and .key file inside $ZIP_PATH" >&2
	exit 1
fi

echo "Found $(basename "$PEM_FILE") + $(basename "$KEY_FILE")"

ssh "$SERVER_USER@$SERVER_HOST" "mkdir -p $REMOTE_CERTS_DIR && chmod 700 $REMOTE_CERTS_DIR"
scp -p "$PEM_FILE" "$KEY_FILE" "$SERVER_USER@$SERVER_HOST:$REMOTE_CERTS_DIR/"
ssh "$SERVER_USER@$SERVER_HOST" "chmod 600 $REMOTE_CERTS_DIR"/*.key "$REMOTE_CERTS_DIR"/*.pem

echo "Reloading nginx..."
ssh "$SERVER_USER@$SERVER_HOST" "docker exec rumgap-nginx-1 nginx -s reload"

echo "Done."
