#!/usr/bin/env python3
"""Read-only Redmine integration check. Inject REDMINE_API_KEY via kanso secret exec."""
import http.cookiejar
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time
import urllib.request

key = os.environ.get("REDMINE_API_KEY")
if not key:
    raise SystemExit("REDMINE_API_KEY is required")
root = pathlib.Path(__file__).resolve().parent.parent
cache = pathlib.Path.home() / ".cache/tmp/opencode"
cache.mkdir(parents=True, exist_ok=True)
redmine_url = os.environ.get("REDMINE_URL", "http://127.0.0.1:8088")

class FixtureInsufficient(Exception):
    pass

with tempfile.TemporaryDirectory(prefix="kanso-redmine-live-", dir=cache) as tmp:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    env = os.environ.copy()
    env.pop("REDMINE_API_KEY", None)
    env.update(KANSO_BIND=f"127.0.0.1:{port}", KANSO_PUBLIC_URL=base,
               KANSO_DATA_DIR=str(pathlib.Path(tmp) / "data"),
               KANSO_PASSWORD="disposable-redmine-live-test",
               KANSO_OPENCHAMBER_URL="", KANSO_OPENCHAMBER_TOKEN="")
    process = subprocess.Popen([str(root / "target/debug/kanso"), "serve"], cwd=root,
                               env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

        def call(path, method="GET", body=None):
            payload = None if body is None else json.dumps(body).encode()
            request = urllib.request.Request(base + path, data=payload, method=method,
                headers={"Origin": base, "Content-Type": "application/json"})
            with opener.open(request, timeout=75) as response:
                data = response.read()
                return json.loads(data) if data else None

        for attempt in range(100):
            if process.poll() is not None:
                raise RuntimeError("Kanso exited during startup")
            try:
                call("/api/health")
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError("Kanso startup timed out")
        call("/api/auth/login", "POST", {"password": "disposable-redmine-live-test"})
        project = call("/api/projects", "POST", {"key": "LIVE", "name": "Temporary live test"})
        connection = call("/api/integration/redmine", "POST",
                          {"name": "Fixture", "url": redmine_url, "key": key})
        source = connection["id"]
        discovery = call(f"/api/integration/redmine/{source}/discover")
        assert discovery["account"]["id"] > 0
        assert len(discovery["projects"]) >= 2

        def preview(filters):
            return call("/api/imports/preview", "POST", {
                "project_id": project["id"], "provider": "redmine",
                "source": source, "filters": filters,
            })

        def identities(result):
            return {item["candidate"]["identity"] for item in result["candidates"]}

        common = None
        for upstream_project in discovery["projects"]:
            candidate = {"project": str(upstream_project["id"]), "state": "all"}
            assigned = identities(preview({**candidate, "assignee": "me"}))
            watched = identities(preview({**candidate, "watched": "me"}))
            if assigned and watched:
                common = candidate
                break
        if common is None:
            raise FixtureInsufficient()
        either = identities(preview({**common, "assignee": "me", "watched": "me", "operator": "or"}))
        both = identities(preview({**common, "assignee": "me", "watched": "me", "operator": "and"}))
        assert either == assigned | watched
        assert both == assigned & watched
        assert identities(preview({**common, "watched": "me", "operator": "or"})) == watched
        result = preview({"state": "all"})
        valid = next((item for item in result["candidates"] if item["valid"]), None)
        assert valid is not None
        confirmation = {"project_id": project["id"], "provider": "redmine", "source": source,
                        "preview_id": result["preview_id"], "selected": [valid["index"]]}
        assert call("/api/imports/confirm", "POST", confirmation)[0]["status"] == "created"
        repeated = preview({"state": "all"})
        matching = next(item for item in repeated["candidates"] if item["candidate"]["identity"] == valid["candidate"]["identity"])
        confirmation.update(preview_id=repeated["preview_id"], selected=[matching["index"]])
        assert call("/api/imports/confirm", "POST", confirmation)[0]["status"] == "skipped"
        print("Live Redmine read-only discovery, AND/OR filters, import and deduplication passed.")
    except FixtureInsufficient:
        raise SystemExit("Live Redmine fixture lacks assigned and watched issues in the same project; filter validation is incomplete.") from None
    except Exception as error:
        # Deliberately omit URLs, bodies, exception messages and environment values.
        raise SystemExit(f"Live Redmine test failed ({type(error).__name__}); inspect service availability and permissions privately.") from None
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
