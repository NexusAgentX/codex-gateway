FROM node:24-bookworm

ENV DEBIAN_FRONTEND=noninteractive
ARG CODEX_VERSION=0.142.5
ENV CODEX_VERSION=${CODEX_VERSION}

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      gosu \
      iproute2 \
      iptables \
      jq \
      less \
      mitmproxy \
      procps \
      tcpdump \
      tini \
      vim-tiny \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g "@openai/codex@${CODEX_VERSION}"

RUN groupadd -r mitmproxy \
    && useradd -r -g mitmproxy -m -d /home/mitmproxy -s /usr/sbin/nologin mitmproxy \
    && groupmod -n codexlab node \
    && usermod -l codexlab -d /home/codexlab -m node \
    && mkdir -p /home/codexlab/.codex /home/mitmproxy/.mitmproxy /flows /workspace \
    && chown -R codexlab:codexlab /home/codexlab /workspace \
    && chown -R mitmproxy:mitmproxy /home/mitmproxy /flows

COPY scripts/entrypoint.sh /usr/local/bin/codex-mitm-entrypoint

RUN chmod +x /usr/local/bin/codex-mitm-entrypoint \
    && cat > /etc/profile.d/codex-mitm.sh <<'EOF'
export NODE_EXTRA_CA_CERTS=/home/mitmproxy/.mitmproxy/mitmproxy-ca-cert.pem
export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
export REQUESTS_CA_BUNDLE=/etc/ssl/certs/ca-certificates.crt
export GIT_SSL_CAINFO=/etc/ssl/certs/ca-certificates.crt
export CODEX_HOME=/home/codexlab/.codex
EOF

WORKDIR /workspace

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/codex-mitm-entrypoint"]
