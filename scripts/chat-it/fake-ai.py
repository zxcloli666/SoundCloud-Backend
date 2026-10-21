import json
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

KEY = Path(sys.argv[1]).read_text().strip()
PORT = int(sys.argv[2])
HARMFUL = ("worthless", "i will find you", "nobody wants you")
SEQS = re.compile(r"Seq numbers of the target's REPORTED messages: (\[[^\]]*\])")


def reported_lines(prompt):
    lines = []
    for raw in prompt.splitlines():
        if not raw.startswith("{"):
            continue
        try:
            line = json.loads(raw)
        except ValueError:
            continue
        if line.get("kind") == "REPORTED" and line.get("from") == "target":
            lines.append(line)
    return lines


def judge(prompt):
    found = SEQS.search(prompt)
    allowed = set(json.loads(found.group(1))) if found else set()
    harmful = [
        line["seq"]
        for line in reported_lines(prompt)
        if line["seq"] in allowed
        and any(word in line["text"].lower() for word in HARMFUL)
    ]
    if harmful:
        return {
            "genuine": True,
            "category": "harassment",
            "severity": "high",
            "recommended_action": "delete",
            "confidence": 0.95,
            "quote_seq": sorted(harmful),
            "rationale": "The reported message insults and threatens the reporter.",
        }
    return {
        "genuine": False,
        "category": "none",
        "severity": "low",
        "recommended_action": "none",
        "confidence": 0.9,
        "quote_seq": [],
        "rationale": "The reported message is ordinary music talk.",
    }


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        if not self.path.endswith("/chat/completions"):
            return self.answer(404, {"error": "not found"})
        if self.headers.get("authorization") != f"Bearer {KEY}":
            return self.answer(401, {"error": "bad key"})
        size = int(self.headers.get("content-length", "0"))
        request = json.loads(self.rfile.read(size))
        schema = request.get("response_format", {}).get("json_schema", {})
        if not schema.get("strict"):
            return self.answer(400, {"error": "strict json schema expected"})
        prompt = request["messages"][-1]["content"]
        verdict = judge(prompt)
        content = json.dumps(verdict)
        if verdict["genuine"]:
            content = f"```json\n{content}\n```"
        self.answer(
            200,
            {
                "id": "fake",
                "object": "chat.completion",
                "model": request.get("model"),
                "choices": [
                    {
                        "index": 0,
                        "message": {"role": "assistant", "content": content},
                        "finish_reason": "stop",
                    }
                ],
            },
        )

    def do_GET(self):
        self.answer(200 if self.path == "/health" else 404, {"ok": True})

    def answer(self, status, body):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def log_message(self, *_):
        pass


ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
