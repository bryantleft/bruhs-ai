#!/usr/bin/env python3
"""Own one temporary Postgres cluster for BNLE-65; never use an existing cluster."""
import argparse
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import socket
import subprocess
import tempfile


def private_json(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(value, output)


def command(arguments, **kwargs):
    result = subprocess.run(arguments, text=True, capture_output=True, timeout=60, **kwargs)
    if result.returncode:
        # Passwords are file input; command output contains only synthetic fixture setup.
        raise RuntimeError(result.stderr.strip() or "Postgres fixture command failed")
    return result.stdout.strip()


def start(binary, output):
    binary = binary.resolve(strict=True)
    for name in ("initdb", "pg_ctl", "createdb", "psql", "postgres"):
        if not (binary / name).is_file():
            raise RuntimeError(f"Choose a Postgres bin directory containing {name}")
    if output.exists():
        raise RuntimeError("Choose a new fixture receipt path; existing files are retained")
    root = Path(tempfile.mkdtemp(prefix="bruhs-ticket-pg-"))
    root.chmod(0o700)
    (root / ".bruhs-database-fixture").write_text(root.name)
    password = secrets.token_urlsafe(32)
    descriptor = os.open(root / "password", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as secret:
        secret.write(password)
    data = root / "data"
    unix = root / "socket"
    unix.mkdir(mode=0o700)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    start_attempted = False
    try:
        command([str(binary / "initdb"), "-D", str(data), "--username=postgres", "--encoding=UTF8", "--no-locale", "--auth-local=trust", "--auth-host=scram-sha-256", "--pwfile", str(root / "password")])
        start_attempted = True
        command([str(binary / "pg_ctl"), "-D", str(data), "-l", str(root / "server.log"), "-o", f"-h 127.0.0.1 -p {port} -k {shlex.quote(str(unix))}", "-w", "start"])
        connection = ["-h", str(unix), "-p", str(port), "-U", "postgres"]
        command([str(binary / "createdb"), *connection, "bruhs_readonly_test"])
        command([str(binary / "psql"), *connection, "-d", "bruhs_readonly_test", "-v", "ON_ERROR_STOP=1"], input="""
CREATE TABLE customers(id bigint PRIMARY KEY,name text,subscription text);
INSERT INTO customers VALUES(1,'Ada',NULL),(2,'Lin','pro');
CREATE FUNCTION test_write() RETURNS int LANGUAGE plpgsql AS $$
BEGIN UPDATE customers SET name='bad'; RETURN 1; END $$;
""")
        receipt = {"name": root.name, "engine": "native", "pg_bin": str(binary), "root": str(root), "data_dir": str(data), "socket_dir": str(unix), "port": port, "database": "bruhs_readonly_test", "admin_user": "postgres", "admin_password": password, "version": command([str(binary / "postgres"), "--version"])}
        private_json(output, receipt)
        print(json.dumps({"fixture": str(output), "engine": receipt["version"], "port": port}))
    except Exception as setup_error:
        if start_attempted:
            try:
                if not (data / "postmaster.pid").exists():
                    raise RuntimeError("Startup did not confirm a server identity")
                command([str(binary / "pg_ctl"), "-D", str(data), "-m", "fast", "-w", "stop"])
            except Exception as cleanup_error:
                recovery = {"name": root.name, "engine": "native", "pg_bin": str(binary), "root": str(root), "data_dir": str(data), "socket_dir": str(unix), "port": port, "database": "bruhs_readonly_test", "admin_user": "postgres", "admin_password": password, "state": "cleanup_required"}
                if not output.exists():
                    private_json(output, recovery)
                raise RuntimeError(f"Fixture setup failed and stop could not be confirmed; retained {root} and recovery receipt {output}. Use pg_ctl with this owned data directory before removing it: {cleanup_error}") from setup_error
        shutil.rmtree(root)
        raise


def stop(path, remove):
    receipt = json.loads(path.read_text())
    root = Path(receipt["root"]).resolve(strict=True)
    if receipt.get("engine") != "native" or not root.name.startswith("bruhs-ticket-pg-"):
        raise RuntimeError("This is not an owned native database fixture")
    if receipt["name"] != root.name or (root / ".bruhs-database-fixture").read_text() != root.name:
        raise RuntimeError("The fixture ownership marker does not match")
    if Path(receipt["data_dir"]).resolve() != root / "data" or Path(receipt["socket_dir"]).resolve() != root / "socket":
        raise RuntimeError("The fixture paths do not match its private root")
    if (root / "data/postmaster.pid").exists():
        command([str(Path(receipt["pg_bin"]) / "pg_ctl"), "-D", str(root / "data"), "-m", "fast", "-w", "stop"])
    if remove:
        # Only the marker-checked directory created by start(), never the caller's project.
        shutil.rmtree(root)
        path.unlink()
    print(json.dumps({"stopped": receipt["name"], "removed": remove}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    create = actions.add_parser("start")
    create.add_argument("--pg-bin", type=Path, required=True)
    create.add_argument("--out", type=Path, required=True)
    end = actions.add_parser("stop")
    end.add_argument("receipt", type=Path)
    end.add_argument("--remove", action="store_true")
    args = parser.parse_args()
    if args.action == "start":
        start(args.pg_bin, args.out)
    else:
        stop(args.receipt, args.remove)


if __name__ == "__main__":
    main()
