"""Собирает dist/out/nebula-client-<версия>-linux-<arch>.tar.gz из готовых бинарников.

Бинарники собираются заранее, на Linux обычным cargo, из Windows или macOS через cargo-zigbuild:
    cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.28
    cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.28
    python dist/linux/package.py
"""
import io
import os
import re
import tarfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
HERE = os.path.join(ROOT, "dist", "linux")
VERSION = re.search(r'^version\s*=\s*"([^"]+)"', open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read(), re.M).group(1)
TARGETS = {"x86_64": "x86_64-unknown-linux-gnu", "aarch64": "aarch64-unknown-linux-gnu"}
FILES = [  # (откуда, имя в архиве, права)
    ("{bin}/nebula-client", "nebula-client", 0o755),
    ("{bin}/nebula-client-service", "nebula-client-service", 0o755),
    (HERE + "/install.sh", "install.sh", 0o755),
    (HERE + "/uninstall.sh", "uninstall.sh", 0o755),
    (HERE + "/nebula-client.service", "nebula-client.service", 0o644),
    (HERE + "/nebula-client.desktop", "nebula-client.desktop", 0o644),
    (HERE + "/nebula-client.png", "nebula-client.png", 0o644),
    (ROOT + "/LICENSE", "LICENSE", 0o644),
]


def main():
    out_dir = os.path.join(ROOT, "dist", "out")
    os.makedirs(out_dir, exist_ok=True)
    for arch, triple in TARGETS.items():
        bin_dir = os.path.join(ROOT, "target", triple, "release")
        if not os.path.exists(os.path.join(bin_dir, "nebula-client-service")):
            print("нет сборки для", triple, "- пропускаю")
            continue
        name = f"nebula-client-{VERSION}-linux-{arch}"
        path = os.path.join(out_dir, name + ".tar.gz")
        with tarfile.open(path, "w:gz") as tar:
            for src, dst, mode in FILES:
                data = open(src.format(bin=bin_dir), "rb").read()
                if dst.endswith((".sh", ".service", ".desktop")):
                    data = data.replace(b"\r\n", b"\n")  # из Windows-чекаута
                info = tarfile.TarInfo(f"{name}/{dst}")
                info.size, info.mode, info.mtime = len(data), mode, int(os.path.getmtime(src.format(bin=bin_dir)))
                tar.addfile(info, io.BytesIO(data))
        print(path)


if __name__ == "__main__":
    main()
