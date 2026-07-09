#!/usr/bin/env bash
set -euo pipefail

MITM_HOME=/home/mitmproxy
MITM_CONF="$MITM_HOME/.mitmproxy"
FLOWS_DIR=/flows
FLOW_FILE="$FLOWS_DIR/codex.mitm"
PCAP_FILE="$FLOWS_DIR/codex.pcap"
CAPTURE_PORTS="${MITM_CAPTURE_PORTS:-80,443}"
BLOCK_HOSTS="${MITM_BLOCK_HOSTS:-}"
BLOCK_IP="${MITM_BLOCK_IP:-203.0.113.1}"

install_ca() {
  mkdir -p "$MITM_CONF" "$FLOWS_DIR"
  chown -R codexlab:codexlab /home/codexlab /workspace
  chown -R mitmproxy:mitmproxy "$MITM_HOME" "$FLOWS_DIR"
  chmod 0777 "$FLOWS_DIR"

  if [[ ! -f "$MITM_CONF/mitmproxy-ca-cert.pem" ]]; then
    gosu mitmproxy mitmdump \
      --set "confdir=$MITM_CONF" \
      --listen-host 127.0.0.1 \
      --listen-port 18080 >/tmp/mitmproxy-ca-bootstrap.log 2>&1 &
    local bootstrap_pid=$!
    for _ in $(seq 1 80); do
      [[ -f "$MITM_CONF/mitmproxy-ca-cert.pem" ]] && break
      sleep 0.1
    done
    kill "$bootstrap_pid" 2>/dev/null || true
    wait "$bootstrap_pid" 2>/dev/null || true
  fi

  if [[ ! -f "$MITM_CONF/mitmproxy-ca-cert.pem" ]]; then
    cat /tmp/mitmproxy-ca-bootstrap.log >&2 || true
    echo "failed to generate mitmproxy CA" >&2
    exit 1
  fi

  if [[ -f "$MITM_CONF/mitmproxy-ca-cert.cer" ]]; then
    cp "$MITM_CONF/mitmproxy-ca-cert.cer" /usr/local/share/ca-certificates/mitmproxy-ca-cert.crt
  else
    cp "$MITM_CONF/mitmproxy-ca-cert.pem" /usr/local/share/ca-certificates/mitmproxy-ca-cert.crt
  fi
  update-ca-certificates >/dev/null
}

setup_iptables() {
  local mitm_uid
  mitm_uid="$(id -u mitmproxy)"

  iptables -t nat -N CODEX_MITM 2>/dev/null || true
  iptables -t nat -F CODEX_MITM
  while iptables -t nat -D OUTPUT -p tcp -j CODEX_MITM 2>/dev/null; do :; done

  iptables -t nat -A OUTPUT -p tcp -j CODEX_MITM
  iptables -t nat -A CODEX_MITM -m owner --uid-owner "$mitm_uid" -j RETURN
  iptables -t nat -A CODEX_MITM -d 127.0.0.0/8 -j RETURN
  iptables -t nat -A CODEX_MITM -d 169.254.0.0/16 -j RETURN
  iptables -t nat -A CODEX_MITM -p tcp -m multiport --dports "$CAPTURE_PORTS" -j REDIRECT --to-ports 8080

  while iptables -D OUTPUT -p udp --dport 443 -j REJECT 2>/dev/null; do :; done
  iptables -A OUTPUT -p udp --dport 443 -j REJECT

  iptables -N CODEX_WALL 2>/dev/null || true
  iptables -F CODEX_WALL
  while iptables -D OUTPUT -j CODEX_WALL 2>/dev/null; do :; done
  iptables -A OUTPUT -j CODEX_WALL
  if [[ -n "$BLOCK_HOSTS" ]]; then
    iptables -A CODEX_WALL -p tcp -d "$BLOCK_IP" -j DROP
  fi
}

setup_blocked_hosts() {
  if [[ -z "$BLOCK_HOSTS" ]]; then
    return
  fi

  sed -i '/^# codex-gateway-mitm blocked hosts begin$/,/^# codex-gateway-mitm blocked hosts end$/d' /etc/hosts 2>/dev/null || true
  {
    echo "# codex-gateway-mitm blocked hosts begin"
    for host in $BLOCK_HOSTS; do
      host="${host%,}"
      [[ -z "$host" ]] && continue
      echo "$BLOCK_IP $host"
    done
    echo "# codex-gateway-mitm blocked hosts end"
  } >>/etc/hosts
}

install_ca
setup_blocked_hosts
setup_iptables

touch "$FLOW_FILE"
touch "$PCAP_FILE"
chown mitmproxy:mitmproxy "$FLOW_FILE" "$PCAP_FILE"
chmod 0666 "$FLOW_FILE" "$PCAP_FILE"

tcpdump -i any -s 0 -w "$PCAP_FILE" >/tmp/codex-mitm-tcpdump.log 2>&1 &

echo "codex-gateway-mitm: mitmdump is listening on transparent port 8080"
echo "codex-gateway-mitm: redirected TCP destination ports: $CAPTURE_PORTS"
if [[ -n "$BLOCK_HOSTS" ]]; then
  echo "codex-gateway-mitm: blocked hosts are mapped to $BLOCK_IP: $BLOCK_HOSTS"
fi
echo "codex-gateway-mitm: flows are being written to $FLOW_FILE"
echo "codex-gateway-mitm: raw packets are being written to $PCAP_FILE"

exec gosu mitmproxy mitmdump \
  --mode transparent \
  --listen-host 0.0.0.0 \
  --listen-port 8080 \
  --showhost \
  --set "confdir=$MITM_CONF" \
  --set flow_detail=2 \
  -w "$FLOW_FILE"
