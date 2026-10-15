#!/usr/bin/env bash
# The app server's VAPID key pair for the push tests, one fresh `prime256v1` key per run.
set -euo pipefail

vapid_generate() {
    openssl ecparam -name prime256v1 -genkey -noout -out "$1/vapid.pem"
}

vapid_public_b64url() {
    openssl ec -in "$1" -pubout -outform DER 2>/dev/null | tail -c 65 | base64 -w0 | tr '+/' '-_' | tr -d '='
}
