#!/usr/bin/env python3
import json
import os
import re
import sys
import urllib.error
import urllib.request

KEY = re.compile(r"^(spotify\d+)_sp_dc$|^(yandex\d+)_token$|^(yt\d+)_cookies$")
SOURCES = {"spotify": "spotify", "yandex": "yandex", "yt": "youtube"}


def read_env(path):
    values = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                key, value = line.split("=", 1)
                values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def accounts(directory):
    env = read_env(os.path.join(directory, "accounts.env"))
    for key, value in sorted(env.items()):
        match = KEY.match(key)
        if not match:
            continue
        name = next(group for group in match.groups() if group)
        source = SOURCES[re.sub(r"\d+$", "", name)]
        if source == "youtube":
            with open(os.path.join(directory, os.path.basename(value)), encoding="utf-8") as cookies:
                value = cookies.read()
        yield name, source, value


def save(base, token, name, source, secret):
    request = urllib.request.Request(
        f"{base}/admin/video-accounts/{name}",
        data=json.dumps({"source": source, "secret": secret}).encode(),
        method="PUT",
        headers={"content-type": "application/json", "x-admin-token": token},
    )
    try:
        with urllib.request.urlopen(request, timeout=90) as reply:
            return reply.status, ""
    except urllib.error.HTTPError as error:
        try:
            return error.code, json.loads(error.read()).get("error", "")
        except ValueError:
            return error.code, ""


def main():
    if len(sys.argv) != 3 or "ADMIN_TOKEN" not in os.environ:
        sys.exit("usage: ADMIN_TOKEN=... seed_video_accounts.py <accounts dir> <api base url>")
    directory, base = sys.argv[1], sys.argv[2].rstrip("/")
    failed = 0
    for name, source, secret in accounts(directory):
        status, error = save(base, os.environ["ADMIN_TOKEN"], name, source, secret)
        print(f"{name} ({source}): {status} {error}".rstrip())
        failed += status != 200
    sys.exit(1 if failed else 0)


main()
