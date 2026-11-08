#!/usr/bin/env python3
"""The Windows installer must say what is missing before it installs.

This build has no WebView interface: the settings window and every panel are
native, so the WebView2 Runtime is no longer a prerequisite. The Visual C++
redistributable still is: it is a system-wide shared component the package does
not ship, and without it the Server cannot start *after* a successful install,
with nothing on screen explaining why.

The check lives in Inno Setup's Pascal Script, which nothing on a non-Windows
host can compile. So this pins the parts that are easy to get wrong and that a
later edit could quietly drop, while a Windows run is still what proves it
compiles and behaves.

The call checks read only the prerequisite code - from `function ReadWebView2Version` to the end of `InitializeSetup` - with Pascal comments removed, and each routine by its own body. Searching the whole script let the uninstaller's `RegDeleteKeyIncludingSubkeys(HKLM64, ...)` and a comment that names both views satisfy the registry-view requirement with the real calls deleted.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SETUP = ROOT / "platforms/windows/installer/msime_setup.iss"


def strip_pascal_comments(code: str) -> str:
    """Drop `{ }`, `(* *)` and `//` comments, leaving `'...'` string literals (which may contain `//` in a URL) intact."""
    out: list[str] = []
    index = 0
    while index < len(code):
        char = code[index]
        if char == "'":
            end = code.find("'", index + 1)
            end = len(code) if end < 0 else end + 1
            out.append(code[index:end])
            index = end
        elif char == "{":
            end = code.find("}", index + 1)
            index = len(code) if end < 0 else end + 1
            out.append(" ")
        elif code.startswith("(*", index):
            end = code.find("*)", index + 2)
            index = len(code) if end < 0 else end + 2
            out.append(" ")
        elif code.startswith("//", index):
            end = code.find("\n", index)
            index = len(code) if end < 0 else end
        else:
            out.append(char)
            index += 1
    return "".join(out)


def prerequisite_code(script: str) -> str:
    """From `function VCRuntimeInstalledInView` to the `end;` closing `InitializeSetup`, comments removed; empty if either end is missing."""
    start = script.find("function VCRuntimeInstalledInView")
    setup = script.find("function InitializeSetup: Boolean;", max(start, 0))
    if start < 0 or setup < 0:
        return ""
    end = re.compile(r"^end;", re.M).search(script, setup)
    if end is None:
        return ""
    return strip_pascal_comments(script[start : end.end()])


def routine(code: str, name: str) -> str:
    """The text of one top-level function in `code`, from its header to its closing `end;`."""
    match = re.search(rf"^function {name}\b.*?^end;", code, re.M | re.S)
    return match.group(0) if match else ""


def main() -> int:
    script = SETUP.read_text(encoding="utf-8")
    failures: list[str] = []

    def require(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)

    require("function InitializeSetup: Boolean;" in script,
            "no InitializeSetup: nothing runs before the first wizard page")
    code = prerequisite_code(script)
    require(code != "", "no VCRuntimeInstalledInView ... InitializeSetup block to check")
    view = routine(code, "VCRuntimeInstalledInView")
    installed = routine(code, "VCRuntimeIsInstalled")
    setup = routine(code, "InitializeSetup")

    # Registry, not the filesystem. Setup.exe is a 32-bit process and Pascal
    # Script's FileExists is redirected to SysWOW64, so probing for
    # vcruntime140.dll reports the x86 runtime and calls an x64-only machine
    # unequipped. Both registry views are named explicitly instead.
    require("Software\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64" in script,
            "VC runtime is not read from its x64 registry key")
    require(re.search(r"FileExists\([^)]*vcruntime", script, re.I) is None,
            "VC runtime probed through the filesystem, which WOW64 redirects")
    for root in ("HKLM32", "HKLM64"):
        require(re.search(rf"VCRuntimeInstalledInView\(\s*{root}\s*\)", installed) is not None,
                f"VCRuntimeIsInstalled does not consult the {root} view")
    require(re.search(r"\bVCRuntimeIsInstalled\b", setup) is not None,
            "InitializeSetup does not check the VC runtime")
    # 这个产品不带 WebView 界面：安装不得再要求 WebView2 Runtime。
    require("WebView2" not in setup,
            "InitializeSetup still mentions the WebView2 Runtime")

    # Installed=1 is true on a machine with only the 2015/2017 redistributable,
    # which lacks vcruntime140_1.dll and still cannot start the Server.
    require(re.search(r"Minor\s*>=\s*20", view) is not None,
            "VC runtime accepted without requiring 14.20 or newer")
    require(re.search(r"RegQueryDWordValue\(\s*RootKey\s*,\s*VCRuntimeKey\s*,\s*'Installed'", view) is not None,
            "VC runtime Installed value is not read")

    # A silent install is CI or a fleet deployment; a modal dialog there is a
    # hang. Default to continuing, with the missing component in the log.
    require("SuppressibleMsgBox" in setup,
            "prerequisite prompt would block a silent install")
    require(re.search(r"SuppressibleMsgBox\([^;]*IDNO\)", setup, re.S) is not None,
            "silent install does not default to continuing")
    require(setup.count("Log('Prerequisite missing:") == 1,
            "the missing VC runtime must be recorded in the install log")

    # The install is not blocked: a user may be about to fetch the component.
    require("MB_YESNO" in setup, "prerequisite prompt is not a yes/no choice")

    require("https://learn.microsoft.com/zh-cn/cpp/windows/latest-supported-vc-redist" in script,
            "missing VC redist download page")
    require("webview2" not in strip_pascal_comments(script).lower(),
            "the installer still requires the WebView2 Runtime")
    require(re.search(r"'http://", script) is None,
            "a download page is offered over plain http")

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    print("installer prerequisites: only the VC runtime is checked before install; WebView2 is not required")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
