#!/usr/bin/env python3
"""按版本资源锁下载桌面词库，校验长度和 SHA-256 后放进安装包的暂存目录。

`resources/desktop-dictionary.lock.json` 是所有平台共用的资源事实源，
`resources/editions/<版本>.lock.json` 是各版本的机械子集（`scripts/editions.py` 生成）。
`Prepare-PackageFiles.ps1` 的 `-DesktopResourcesDirectory`（默认
`target/desktop-resources`）要求目录里的文件与锁逐一对应，所以这里保持平面布局，
每个锁条目落到 `<out>/<name>`。

已存在且摘要匹配的文件不重复下载；锁里没有的文件原样保留，方便在同一个目录
准备多个版本的资源。下载先写临时文件，校验通过后才替换目标，失败不会留下
看起来已完成的内容。

usage: fetch_desktop_resources.py [--edition pinyin] [--out target/desktop-resources]
"""
import argparse
import hashlib
import json
import re
import sys
import tempfile
from pathlib import Path

import download_retry

ROOT = Path(__file__).resolve().parents[1]
FULL_LOCK = ROOT / "resources/desktop-dictionary.lock.json"
DEFAULT_OUT = ROOT / "target/desktop-resources"
CHUNK = 1 << 20
NAME_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")


def digest(path: Path) -> str:
    sha = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(CHUNK), b""):
            sha.update(block)
    return sha.hexdigest()


def matches(path: Path, artifact: dict) -> bool:
    return path.is_file() and path.stat().st_size == artifact["size"] and digest(path) == artifact["sha256"]


def check_lock(lock: dict) -> None:
    if not isinstance(lock.get("artifacts"), list) or not lock["artifacts"]:
        raise SystemExit("资源锁里没有 artifacts")
    seen = set()
    for artifact in lock["artifacts"]:
        name = artifact.get("name")
        if not isinstance(name, str) or not NAME_PATTERN.fullmatch(name):
            raise SystemExit(f"资源锁里的文件名不安全：{name!r}")
        if name in seen:
            raise SystemExit(f"资源锁重复列出 {name}")
        seen.add(name)
        if not isinstance(artifact.get("size"), int) or artifact["size"] <= 0:
            raise SystemExit(f"{name}: 锁里的长度无效")
        if not re.fullmatch(r"[a-f0-9]{64}", str(artifact.get("sha256", ""))):
            raise SystemExit(f"{name}: 锁里的 SHA-256 无效")
        if not str(artifact.get("url", "")).startswith("https://"):
            raise SystemExit(f"{name}: 拒绝非 HTTPS 来源")


def download(artifact: dict, destination: Path) -> None:
    print(f"  下载 {artifact['name']}（{artifact['size'] / 1e6:.1f} MB）", file=sys.stderr)
    destination.parent.mkdir(parents=True, exist_ok=True)
    # 先写同目录的临时文件：Windows 不允许删除仍打开的文件，所以关闭后再改名或清理。
    staged = tempfile.NamedTemporaryFile(dir=destination.parent, delete=False)
    staged_path = Path(staged.name)
    try:
        with staged:
            with download_retry.urlopen(artifact["url"], timeout=300) as response:
                advertised = response.headers.get("Content-Length")
                if advertised is not None:
                    try:
                        if int(advertised) > artifact["size"]:
                            raise SystemExit(f"{artifact['name']}: 响应比锁里记录的长")
                    except ValueError:
                        pass
                size = 0
                while block := response.read(CHUNK):
                    size += len(block)
                    if size > artifact["size"]:
                        raise SystemExit(f"{artifact['name']}: 响应比锁里记录的长")
                    staged.write(block)
    except BaseException:
        staged_path.unlink(missing_ok=True)
        raise
    actual = digest(staged_path)
    size = staged_path.stat().st_size
    if actual != artifact["sha256"] or size != artifact["size"]:
        staged_path.unlink(missing_ok=True)
        raise SystemExit(
            f"{artifact['name']}: 期望 {artifact['sha256']}、{artifact['size']} 字节，"
            f"实际 {actual}、{size} 字节"
        )
    staged_path.replace(destination)


def install(edition: str, out: Path) -> Path:
    lock_path = FULL_LOCK if edition == "full" else ROOT / f"resources/editions/{edition}.lock.json"
    if not lock_path.is_file():
        raise SystemExit(f"没有版本 {edition} 的资源锁：{lock_path}")
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    check_lock(lock)
    out.mkdir(parents=True, exist_ok=True)
    for artifact in lock["artifacts"]:
        destination = out / artifact["name"]
        if matches(destination, artifact):
            print(f"  {artifact['name']}: 已是锁定的摘要", file=sys.stderr)
            continue
        download(artifact, destination)
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--edition", default="pinyin", help="shared/contracts/editions.json 里的版本 id，默认 pinyin")
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    arguments = parser.parse_args()
    print(install(arguments.edition, arguments.out.resolve()))


if __name__ == "__main__":
    main()
