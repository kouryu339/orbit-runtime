import json
import socketserver
import sys


def send_line(wfile, message):
    wfile.write((json.dumps(message, ensure_ascii=False) + "\n").encode("utf-8"))
    wfile.flush()


def read_line(rfile):
    line = rfile.readline()
    if not line:
        raise RuntimeError("connection closed")
    return json.loads(line.decode("utf-8"))


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        try:
            message = read_line(self.rfile)
            if message.get("type") != "execute":
                send_line(self.wfile, {"type": "error", "message": "expected execute"})
                return

            request = message["request"]
            args_cli = request.get("args_cli", "")
            value = "hello-from-python"
            parts = args_cli.split()
            if "--value" in parts:
                idx = parts.index("--value")
                if idx + 1 < len(parts):
                    value = parts[idx + 1]

            send_line(
                self.wfile,
                {
                    "type": "ai_output",
                    "output": {
                        "result": {"echoed_value": value},
                        "to_ai": f"Python RPC echoed value {value!r}.",
                        "error_code": 0,
                    },
                },
            )
        except Exception as exc:
            send_line(self.wfile, {"type": "error", "message": str(exc)})


def main():
    host = "127.0.0.1"
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 58081
    with socketserver.ThreadingTCPServer((host, port), Handler) as server:
        print(f"python rpc tool server listening on {host}:{port}", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
