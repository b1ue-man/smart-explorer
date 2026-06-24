# Share-Server Signaling

`se-share-server` is an untrusted rendezvous server. It routes signed presence
and room/direct events only; file operations still run directly between peers.

## Transports

- `host:51820` or `tcp://host:51820`: raw newline-delimited TCP signaling.
- `ws://host/path`: WebSocket signaling without TLS.
- `wss://host/path`: WebSocket signaling through TLS, intended for port 443.
- `https://host/path` in the app is treated as `wss://host/path`.

Multiple app endpoints can be separated with commas or semicolons:

```text
wss://share.example.com/se-share, share.example.com:51820
```

## HTTPS / 443 Deployment

Run the server locally and terminate TLS in a reverse proxy:

```text
se-share-server 127.0.0.1:51820
```

Caddy example:

```caddyfile
share.example.com {
    reverse_proxy /se-share 127.0.0.1:51820
}
```

Nginx example:

```nginx
location /se-share {
    proxy_pass http://127.0.0.1:51820;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
}
```

This improves the signaling path on restrictive networks. It does not relay peer
file traffic; if two devices cannot open a direct TCP path, the peer connection
will still show as not directly reachable until relay/TURN support exists.
