"""Exercise the real shape gate on isolated tracked-content copies."""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]


def main():
    binary = pathlib.Path(sys.argv[1]).resolve()
    runtime = ROOT / ".ynventa/materialized"
    runtime.mkdir(parents=True, exist_ok=True)
    tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    with tempfile.TemporaryDirectory(prefix="shape-check-", dir=runtime) as directory:
        fixture = pathlib.Path(directory).resolve()
        assert fixture.is_relative_to(runtime.resolve())
        for name in filter(None, tracked):
            source = ROOT / name
            if source.is_file():
                destination = fixture / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, destination)

        def check(expected):
            run = subprocess.run([str(binary), str(fixture)], capture_output=True, text=True)
            report = json.loads(run.stdout)
            assert run.returncode == expected, (run.returncode, report, run.stderr)
            return report

        assert check(0)["status"] == "PASS"
        paths = ["crate/probe.rs", "crates/probe.rs", "adapter/web/src/crates/probe.rs"]
        for name in paths:
            probe = fixture / name
            probe.parent.mkdir(parents=True, exist_ok=True)
            probe.write_text("// Deliberately invalid architecture fixture.\n", encoding="utf-8")
            assert name in check(1)["forbidden_architecture_paths"]
            probe.unlink()
            probe.parent.rmdir()

        declaration = fixture / ".ynventa/declared/repository.rs"
        original = declaration.read_text(encoding="utf-8")
        edge = 'Edge { from: "commerce.web-research", to: "commerce", kind: EdgeKind::DependsOn, scope: Scope::Runtime },'
        assert edge in original
        declaration.write_text(original.replace(edge, "", 1), encoding="utf-8")
        report = check(1)
        assert any(e["from_owner"] == "commerce.web-research" for e in report["undeclared_cargo_dependencies"])
        declaration.write_text(original, encoding="utf-8")
        assert check(0)["status"] == "PASS"
    print("PASS: baseline, root crate/crates, nested crates, undeclared Cargo edge, restored baseline")


if __name__ == "__main__":
    main()
