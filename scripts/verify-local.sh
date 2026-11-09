#!/usr/bin/env bash
# 水杉输入法 Windows 拼音输入法的本地门禁。
#
# 这是 GitHub Actions 覆盖不到的那一半，也是提交前的快速反馈：Rust workspace 的编译、测试、fmt、clippy 和依赖审计，Windows C++ 宿主的交叉编译、原生测试和契约检查。全部都在本机跑完并汇总结果。
#
# 这套门禁的意义在于基线。几个套件有长期存在的失败，所以单看通过/失败的数字没有意义：合并前唯一要回答的问题是「这次改动有没有弄坏本来能用的东西」。因此每个阶段都把失败测试的**名字**与 scripts/known-failures.txt 比较，只有基线之外的名字才判失败。
#
# 六次编译中断曾在同一个会话里进入 develop，都是因为分支合并后没有编译合并结果。`--quick` 就是为这个准备的：它只跑编译阶段。
#
# 用法：
#   scripts/verify-local.sh --quick   只编译，合并前的门禁
#   scripts/verify-local.sh           全部
#   scripts/verify-local.sh --update-baseline   把这次观察到的失败追加到 known-failures.txt
#
# `--quick` 跳过本机没有工具链或构建目录的阶段；这些阶段会打印跳过原因，不打印就是真的跑了。全套运行不会因为环境跳过 Rust 编译、契约检查或 Windows 原生测试；只有需要 Wine、vcpkg 或 MinGW 的 Windows 构建阶段按环境跳过。MSIME_VERIFY_ALL 不再有额外含义，保留只是为了让已有的调用脚本继续工作。
#
# `--update-baseline` 只追加，不删除。部分测试不稳定，单次运行会漏报：提交的基线是多次运行的并集，修好后由人工删除条目。
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
baseline="scripts/known-failures.txt"
quick=0
update=0
for argument in "$@"; do
  case "$argument" in
    --quick) quick=1 ;;
    --update-baseline) update=1 ;;
    *) echo "unknown option: $argument" >&2; exit 2 ;;
  esac
done

# vcpkg 提供 Windows 宿主链接的 curl、fmt、nlohmann-json 和 utfcpp。优先从 VCPKG_ROOT 推导，否则用安装好的目录设置 MSIME_VCPKG_PREFIX。两者都没有时 CMake 会在看起来像代码错误的地方失败，所以下面的 Windows 分支会直接说明原因，而不是让人从编译器消息里猜。
: "${MSIME_VCPKG_PREFIX:=${VCPKG_ROOT:+$VCPKG_ROOT/installed/x64-windows-static-md}}"
: "${MSIME_NATIVE_BUILD:=target/win-full}"
# 无 Rust 库的协议测试配置和主构建分开，否则没人会跑到它，配置也会烂掉。
: "${MSIME_PIPE_BUILD:=target/windows-pipe}"
# 不带数字的 `cmake --build --parallel` 在 Makefile 生成器下就是不限并发的 `make -j`。每次 cmake 构建都带上这个数：先取 `CMAKE_BUILD_PARALLEL_LEVEL`，没有就跟 `CARGO_BUILD_JOBS`，再没有才用本机核数。
: "${MSIME_BUILD_JOBS:=${CMAKE_BUILD_PARALLEL_LEVEL:-${CARGO_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}}}"
export MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*'

windows_host=0
case "$(uname -s 2>/dev/null)" in MINGW*|MSYS*|CYGWIN*) windows_host=1 ;; esac

# Windows 的编译门禁也可以在没有 Windows 宿主的机器上通过 MinGW 交叉构建运行。它需要编译器和已经 bootstrap 到 manifest 基线的 vcpkg；bootstrap 是漫长的下载，所以这里只采用已经存在的树。
cross_vcpkg=""
if [ "$windows_host" -eq 0 ] && command -v x86_64-w64-mingw32-g++ >/dev/null 2>&1; then
  main_worktree="$(dirname "$(git rev-parse --git-common-dir 2>/dev/null || echo .)")"
  for candidate in "${MSIME_VCPKG_ROOT:-}" "$root/target/tooling/vcpkg" \
    "$main_worktree/target/tooling/vcpkg"; do
    [ -n "$candidate" ] && [ -x "$candidate/vcpkg" ] && cross_vcpkg="$candidate" && break
  done
fi

# 只有 Windows 宿主才从 vcpkg 取 CMake 搜索路径；别的机器上导出会覆盖 README 让用户设置的路径。
if [ "$windows_host" -eq 1 ]; then
  if [ -n "$MSIME_VCPKG_PREFIX" ]; then
    export CMAKE_PREFIX_PATH="$MSIME_VCPKG_PREFIX"
    export CXXFLAGS="-I$MSIME_VCPKG_PREFIX/include"
  else
    echo "note: neither MSIME_VCPKG_PREFIX nor VCPKG_ROOT is set; the Windows build will not find its vcpkg dependencies"
  fi
fi

failed=0
new_failures=""
observed_wine=""
collected=""
# 临时文件与构建锁共用退出清理，申请失败时也能回收已经创建的文件。
held_lock=""
# shellcheck disable=SC2317  # reached through the trap
cleanup() {
  local file
  # 空收集器不能展开为 .*，否则会删除 worktree 的 .git 等隐藏文件。
  if [ -n "$collected" ]; then
    for file in "$collected" "$collected".*; do
      rm -f -- "$file"
    done
  fi
  for file in "$observed_wine" "${cross_log:-}" "${wine_log:-}"; do
    if [ -n "$file" ]; then rm -f -- "$file"; fi
  done
  if [ -n "$held_lock" ]; then rmdir "$held_lock" 2>/dev/null; fi
  # The contract checks' output directory, made by scripts/run-checks.sh when it is sourced below.
  if [ -n "${checks_dir:-}" ]; then rm -rf -- "$checks_dir"; fi
  return 0
}
trap cleanup EXIT
if ! observed_wine="$(mktemp)"; then
  echo "FAIL: cannot create verification temporary file" >&2
  exit 1
fi
if ! collected="$(mktemp)"; then
  echo "FAIL: cannot create verification collector" >&2
  exit 1
fi
note() { printf '\n=== %s ===\n' "$1"; }
fail() { echo "FAIL: $1"; failed=1; }

# Compare this phase's failing test names against the baseline. Anything not
# already known is a regression this change introduced.
compare() {
  local phase="$1" observed="$2"
  local unexpected
  unexpected="$(comm -23 <(grep -v '^ *$' "$observed" | sort -u)                         <(grep -vE '^ *(#|$)' "$baseline" | sort -u) || true)"
  if [ -n "$unexpected" ]; then
    fail "$phase has failures that are not in the baseline:"
    printf '  %s\n' "$unexpected"
    new_failures="$new_failures$unexpected"$'\n'
  else
    echo "$phase: at baseline"
  fi
}

# The special checks below run inline here because they need this script's toolchains; scripts/run-checks.sh names them in special_checks, and its registry phase fails when this file stops invoking them. Everything else under scripts/test-*.py is discovered and run by that script, sourced further down.

# 32 位 TSF DLL 会被每个 32 位宿主应用加载，而没有任何地方编译这个架构：build-cross.sh x86 需要 DWARF 展开的 MinGW，常见的 macOS 工具链是 SJLJ。这里用同一个 i686 编译器和同一批来自 x64 构建的标志检查同一份源码，不链接，所以不需要 32 位库。
note "windows x86 syntax"
python3 scripts/test-windows-32bit-compile.py || fail "windows x86 syntax"

# Most of the Windows tests are policy with no Win32 call in the translation
# unit. Same sources, host compiler, actually executed.
note "windows tests on this host"
python3 scripts/test-windows-native-run.py || fail "windows tests on this host"

# The contract checks themselves: the registry phase and every other scripts/test-*.py, discovered by file name. Sourced rather than run so the output and this script's failure count stay as they were; the contracts workflow runs the same file on its own. It has to stay after the two inline special phases just above.
# shellcheck source=scripts/run-checks.sh
. scripts/run-checks.sh

# The quality workflow runs actionlint on every pull request, so a workflow-only change used to be verified here by every platform phase and by nothing that reads YAML. Same tool and the same shellcheck severity as that workflow; skipped where actionlint is not installed.
note "workflows: actionlint"
if command -v actionlint >/dev/null 2>&1; then
  if SHELLCHECK_OPTS=--severity=warning actionlint -no-color; then
    echo "actionlint: workflows are valid"
  else
    fail "actionlint"
  fi
else
  echo "skipped: actionlint not installed (the quality workflow runs it in CI)"
fi

note "agent notes"
# 笔记本身是散文，但树结构、头块格式和归档封印都可机械校验，烂掉的笔记会让约定静默腐化。与 `pnpm run verify-notes` 同一组检查。
if ! command -v node >/dev/null 2>&1; then
  echo "skipped: node not installed (pnpm run verify-notes runs the same checks)"
elif ! node -e 'process.exit(process.features.typescript ? 0 : 1)' 2>/dev/null; then
  echo "skipped: node too old to execute TypeScript directly (need 22.18+ / 23.6+)"
elif [ ! -d "$root/.agents/notes" ]; then
  fail "agent notes: .agents/notes is missing"
else
  # 归档封印的 append-only 基准必须是与 origin/develop 的分叉点；拿不到分叉点时脚本自动降级为只对磁盘内容做封印自校验。
  agent_base="$(git merge-base origin/develop HEAD 2>/dev/null || true)"
  # Windows Node 不转换环境变量里的 MSYS 路径，否则会把笔记树当成不存在而空扫。
  if [ "$windows_host" -eq 1 ]; then
    export AGENT_NOTE_ROOT="$(cygpath -w "$root/.agents/notes")"
  else
    export AGENT_NOTE_ROOT="$root/.agents/notes"
  fi
  if [ -n "$agent_base" ]; then
    export AGENT_NOTE_ARCHIVE_BASE_REF="$agent_base"
  fi
  if node .agents/skills/write-notes-like-deepseek/scripts/verify-agent-note-tree.ts \
    && node .agents/skills/write-notes-like-deepseek/scripts/verify-agent-note-format.ts \
    && node .agents/skills/write-notes-like-deepseek/scripts/verify-archived-agent-notes.ts; then
    echo "agent notes: tree, format and seals verified"
  else
    fail "agent notes"
  fi
fi

note "compile: rust workspace"
cargo check --workspace --all-targets 2>&1 | tail -3
[ "${PIPESTATUS[0]}" -eq 0 ] || fail "cargo check"

# Windows 原生配置可能链接显式 MSVC 目标，而本机默认 Rust 是 GNU；必须重建缓存里真正链接的那一份。
build_windows_host_library() {
  local library profile_dir cargo_dir cargo_root profile target runtime build_status cargo_path msvc_tools host_arch target_arch linker linker_key
  local target_args=() cargo_env=()
  library=$(sed -n 's/^MSIME_HOST_LIBRARY:[A-Z]*=//p' "$MSIME_NATIVE_BUILD/CMakeCache.txt" 2>/dev/null | head -1)
  if [ -z "$library" ]; then
    echo "native: $MSIME_NATIVE_BUILD/CMakeCache.txt names no MSIME_HOST_LIBRARY"
    return 1
  fi
  if command -v cygpath >/dev/null 2>&1; then
    library=$(cygpath -u "$library") || return 1
  fi
  profile_dir=$(dirname "$library")
  cargo_dir=$(cd "$(dirname "$profile_dir")" && pwd -P) || return 1
  cargo_root=$(cd target && pwd -P) || return 1
  if [ "$cargo_dir" != "$cargo_root" ]; then
    target=$(basename "$cargo_dir")
    if [ "$(dirname "$cargo_dir")" != "$cargo_root" ]; then
      echo "native: configured host library is outside this checkout's target directory: $library"
      return 1
    fi
    case "$target" in
      *-pc-windows-msvc|*-pc-windows-gnu) target_args=(--target "$target") ;;
      *) echo "native: unsupported Rust target directory: $cargo_dir"; return 1 ;;
    esac
  fi
  profile=$(basename "$profile_dir")
  [ "$profile" = debug ] && profile=dev
  cargo_path="$PATH"
  if [ -n "${VCToolsInstallDir:-}" ]; then
    msvc_tools="$VCToolsInstallDir"
    if command -v cygpath >/dev/null 2>&1; then msvc_tools=$(cygpath -u "$msvc_tools") || return 1; fi
    host_arch=${VSCMD_ARG_HOST_ARCH:-x64}
    target_arch=${VSCMD_ARG_TGT_ARCH:-x64}
    [ "$host_arch" = amd64 ] && host_arch=x64
    [ "$target_arch" = amd64 ] && target_arch=x64
    msvc_tools="${msvc_tools%/}/bin/Host$host_arch/$target_arch"
    # Git Bash 的 link.exe 是硬链接工具；仅给 Cargo 调整顺序，其他脚本仍使用原来的 Unix 工具。
    if [ -f "$msvc_tools/link.exe" ]; then cargo_path="$msvc_tools:$cargo_path"; fi
  fi
  case "${target:-}" in
    *-pc-windows-msvc)
      linker=$(sed -n 's/^CMAKE_LINKER:[A-Z]*=//p' "$MSIME_NATIVE_BUILD/CMakeCache.txt" | head -1)
      if command -v cygpath >/dev/null 2>&1 && [ -n "$linker" ]; then linker=$(cygpath -u "$linker") || return 1; fi
      [ -n "$linker" ] || linker="${msvc_tools:-}/link.exe"
      if [ ! -f "$linker" ]; then
        echo "native: MSVC linker is unavailable; configure CMAKE_LINKER or initialize the Visual Studio environment"
        return 1
      fi
      # 为实际 Rust 目标传入绝对路径，不能只凭 Bash 的 PATH 推断原生 rustc 会选谁。
      linker_key=$(printf 'CARGO_TARGET_%s_LINKER' "$target" | tr '[:lower:]-' '[:upper:]_')
      if command -v cygpath >/dev/null 2>&1; then linker=$(cygpath -w "$linker") || return 1; fi
      cargo_env=("$linker_key=$linker")
      echo "native: $linker_key=$linker"
      ;;
  esac
  echo "native: rebuilding $library"
  # macOS 系统 Bash 在 nounset 下把空数组视为未设置，可选参数只在数组非空时展开。
  env PATH="$cargo_path" ${cargo_env[@]+"${cargo_env[@]}"} cargo build -p msime-host-api --locked --profile "$profile" ${target_args[@]+"${target_args[@]}"} 2>&1 |
    tee "$MSIME_NATIVE_BUILD/host-library-build.log" | tail -2
  build_status=${PIPESTATUS[0]}
  if [ "$build_status" -ne 0 ]; then
    cat "$MSIME_NATIVE_BUILD/host-library-build.log"
    return 1
  fi
  runtime="$profile_dir/msime_host_api.dll"
  [ -f "$runtime" ] || runtime="$profile_dir/libmsime_host_api.so"
  if [ ! -f "$runtime" ]; then
    echo "native: configured host runtime was not produced: $profile_dir"
    return 1
  fi
  mkdir -p "$MSIME_NATIVE_BUILD/Debug" && cp -f "$runtime" "$MSIME_NATIVE_BUILD/Debug/"
  # add_subdirectory 的子项目把自己的测试放在各自的输出目录里（tsf/Debug），那里同样要有这个运行时，否则它们一个都起不来：Windows 上缺 DLL 的进程会挂到 ctest 的超时，看起来像测试本身卡死。按 configure 就生成的 CTestTestfile.cmake 找出登记了测试的子项目，先建目录再复制，第一次运行也不漏。
  for sub in "$MSIME_NATIVE_BUILD"/*/CTestTestfile.cmake; do
    [ -f "$sub" ] || continue
    sub=$(dirname "$sub")
    mkdir -p "$sub/Debug" && cp -f "$runtime" "$sub/Debug/"
  done
}

note "compile: native host"
if [ -d "$MSIME_NATIVE_BUILD" ]; then
  if build_windows_host_library; then
    cmake --build "$MSIME_NATIVE_BUILD" --config Debug 2>&1 | grep -Ei "error C[0-9]|error LNK" | head -5
    cmake --build "$MSIME_NATIVE_BUILD" --config Debug >/dev/null 2>&1 || fail "native build"
  else
    fail "cargo build -p msime-host-api (configured native host library)"
  fi
elif [ -n "$cross_vcpkg" ]; then
  # Not a Windows host, but the MinGW toolchain and a bootstrapped vcpkg are
  # both here, so the Windows compile gate can run anyway.
  #
  # One cross build per machine at a time. The vcpkg checkout and the built
  # dependencies are shared, so two runs otherwise overlap: one reinstalls the
  # manifest into the prefix the other is already compiling against, and the
  # second fails on a header that is present before and after. A gate that goes
  # red for reasons unrelated to the change is a gate people learn to pass with
  # --no-verify, so a run that cannot take the lock reports what it did.
  #
  # mkdir is the test-and-set: it is atomic on every filesystem this runs on,
  # unlike "test -e then create". The trap covers an interrupted run; a lock
  # left by a killed process is cleared by removing the directory it names,
  # which the message points at.
  cross_deps="$(dirname "$cross_vcpkg")/windows-native-deps"
  mkdir -p "$cross_deps" 2>/dev/null
  cross_lock="$cross_deps/.verify-cross-build.lock"
  if ! mkdir "$cross_lock" 2>/dev/null; then
    echo "windows cross build: skipped (another run holds $cross_lock)"
  else
    held_lock="$cross_lock"
    cross_log="$(mktemp)"
    if MSIME_VCPKG_ROOT="$cross_vcpkg" MSIME_WINDOWS_DEPS_ROOT="$cross_deps" \
      bash platforms/windows/build-cross.sh x64 >"$cross_log" 2>&1; then
      echo "windows cross build (x64): links"
    elif grep -q "Failed to take the filesystem lock" "$cross_log"; then
      # vcpkg's own lock, taken by something that is not this gate.
      echo "windows cross build: skipped (vcpkg busy in another run)"
    else
      grep -Ei "error:|Error [0-9]|No rule to make target" "$cross_log" | head -5
      fail "windows cross build"
    fi
    rm -f "$cross_log"
    rmdir "$cross_lock" 2>/dev/null
    held_lock=""
  fi
else
  echo "skipped: $MSIME_NATIVE_BUILD not configured, and no MinGW cross toolchain"
  echo "  run platforms/windows/build-cross.sh x64 once to enable this gate here"
fi

# Not in --quick: it emulates x86_64 on an arm64 host, so it costs minutes.
# It is the only thing here that runs the Windows suites rather than building
# them, which is why it is in the full run rather than nowhere.
if [ "$quick" -eq 0 ]; then
  note "windows suites under wine"
  wine_log="$(mktemp)"
  bash platforms/windows/run-tests-wine.sh x64 >"$wine_log" 2>&1
  if grep -q "^skipped:" "$wine_log"; then
    sed -n '1,2p' "$wine_log"
  else
    grep "^FAIL " "$wine_log" | sed 's/^FAIL /wine /' > "$observed_wine"
    echo "wine: $(grep -c '^PASS' "$wine_log") passed, $(grep -c '^FAIL ' "$wine_log") failed, $(grep -c '^SKIP ' "$wine_log") skipped"
    # 环境不支持的原生像素套件必须在总门禁中显示原因，不能悄悄消失。
    grep '^SKIP ' "$wine_log" || true
    compare "windows suites under wine" "$observed_wine"
  fi
  rm -f "$wine_log"
fi

note "compile: pipe-only configuration"
# Cheap: no Rust library, no vcpkg dependencies, just the protocol tests.
if cmake -S platforms/windows -B "$MSIME_PIPE_BUILD" -DMSIME_WINDOWS_PIPE_ONLY=ON      >/dev/null 2>&1; then
  if cmake --build "$MSIME_PIPE_BUILD" --config Debug >/dev/null 2>&1; then
    echo "pipe-only: builds"
  else
    fail "pipe-only build"
    cmake --build "$MSIME_PIPE_BUILD" --config Debug 2>&1 |
      grep -Ei "error C[0-9]|error LNK" | head -5
  fi
elif [ "$windows_host" -eq 0 ] && command -v x86_64-w64-mingw32-g++ >/dev/null 2>&1; then
  # It cannot configure against the *host* compiler off Windows, but it can
  # cross-configure, and this one needs nothing but the compiler - no Rust
  # library, no vcpkg.
  for cross_arch in x86_64 i686; do
    command -v "$cross_arch-w64-mingw32-g++" >/dev/null 2>&1 || continue
    cross_dir="${MSIME_PIPE_BUILD}-cross-$cross_arch"
    if cmake -S platforms/windows -B "$cross_dir" -DMSIME_WINDOWS_PIPE_ONLY=ON \
      -DCMAKE_SYSTEM_NAME=Windows -DCMAKE_CXX_COMPILER="$cross_arch-w64-mingw32-g++" \
      -DCMAKE_BUILD_TYPE=Debug >/dev/null 2>&1 &&
      cmake --build "$cross_dir" --parallel "$MSIME_BUILD_JOBS" >/dev/null 2>&1; then
      echo "pipe-only: cross-builds ($cross_arch)"
    else
      cmake --build "$cross_dir" --parallel "$MSIME_BUILD_JOBS" 2>&1 |
        grep -Ei "error:|Error [0-9]" | head -5
      fail "pipe-only cross build ($cross_arch)"
    fi
  done
elif [ "$windows_host" -eq 0 ]; then
  # platforms/windows cannot configure off Windows without a cross compiler.
  echo "pipe-only: skipped (needs a Windows host or a MinGW cross compiler)"
else
  fail "pipe-only configure"
fi

if [ "$quick" -eq 1 ]; then
  echo
  if [ "$failed" -eq 0 ]; then
    echo "quick check passed"
  else
    echo "quick check FAILED"
  fi
  exit "$failed"
fi

note "rust tests"
: > "$collected.rust"
# 用户词库日志改用 WAL 之后，每个还开着的日志除了主文件还占着 `-wal`、`-shm`；msime-engine 与 msime-host-api 的测试并行建几百个临时日志。macOS 上 launchd 给的软上限是 256（经 SSH 的 rbuild 也是），会让夹具随机报 `unable to open database file` / `Too many open files`。这里只抬软上限，硬上限更低时保持原样。
ulimit -n 4096 2>/dev/null || true
# 这里的列表是 workspace 的 default-members 加上 Windows 宿主依赖的全部 crate。不以 cargo 的退出码为准（已知失败会让它非零），一个根本编不过的 crate 收集不到任何失败的测试名，会被报告成处于基线，所以下面单独检查编译错误。
for package in msime-path-trust msime-client-base msime-client-core msime-client-services msime-engine msime-input-runtime msime-host-api; do
  status=0 build_failed=0
  cargo test -p "$package" --no-fail-fast > "$collected.$package" 2>&1 || status=$?
  if grep -qE "^error: (could not compile|linking with)" "$collected.$package"; then
    grep -E "^error: (could not compile|linking with)" "$collected.$package" | head -1
    fail "$package build"
    build_failed=1
  fi
  # Names come only from the list under each binary's second `failures:` header (the first is followed by unindented `---- name stdout ----` blocks), so top-level integration tests and doc-tests count too.
  names="$(awk '/^failures:$/ { list = 1; next } list && /^    / { sub(/^ +/, ""); print; next } { list = 0 }' \
    "$collected.$package")"
  if [ -n "$names" ]; then
    printf '%s\n' "$names" | sed "s#^#$package #" >> "$collected.rust"
  elif [ "$status" -ne 0 ] && [ "$build_failed" -eq 0 ]; then
    # A failure cargo named nowhere (a crashed test binary, a summary this parser missed) is still one.
    echo "$package <unnamed failure>" >> "$collected.rust"
  fi
done
compare "rust tests" "$collected.rust"

note "rust fmt (changed files only)"
# The whole tree is not clean, and reformatting it would collide with every
# other branch in flight. What matters is that this change does not add to it -
# so an unformatted file this change touched is a failure, not a printed note.
changed="$(git diff --name-only origin/develop...HEAD -- '*.rs' 2>/dev/null || true)"
if [ -n "$changed" ]; then
  unformatted=""
  for file in $changed; do
    [ -f "$file" ] || continue
    rustfmt --check --edition 2021 "$file" >/dev/null 2>&1 || unformatted="$unformatted  $file"$'\n'
  done
  if [ -n "$unformatted" ]; then
    fail "these changed files are not rustfmt-clean:"
    printf '%s' "$unformatted"
  else
    echo "changed Rust files: rustfmt-clean"
  fi
else
  echo "no Rust files changed against origin/develop"
fi

note "clippy: first-party crates"
# 下面这些 crate 在 -D warnings 下都是干净的，所以这是没有基线的硬门槛：一旦失败，就是被测的改动引起的。
clippy_failed=""
for crate in msime-path-trust msime-client-base msime-client-core msime-client-services msime-engine msime-input-runtime msime-host-api; do
  cargo clippy -p "$crate" --all-targets -- -D warnings >/dev/null 2>&1 ||
    clippy_failed="$clippy_failed  $crate"$'\n'
done
if [ -n "$clippy_failed" ]; then
  fail "clippy -D warnings failed for:"
  printf '%s' "$clippy_failed"
else
  echo "first-party crates: clippy clean at -D warnings"
fi

note "dependency advisories"
# Lockfile-only, so it runs without building anything. A vulnerability is fatal;
# unmaintained and unsound advisories are accepted one at a time in
# .cargo/audit.toml, each with the chain that pulls it in and why - the same
# discipline as known-failures.txt, for the same reason.
#
# Not part of --quick: that gate has to work offline, and this fetches the
# advisory database.
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit 2>&1 | tail -3
  [ "${PIPESTATUS[0]}" -eq 0 ] || fail "cargo audit"
else
  echo "cargo-audit not installed; skipping (cargo install cargo-audit)"
fi

note "native tests"
if [ -d "$MSIME_NATIVE_BUILD" ]; then
  (cd "$MSIME_NATIVE_BUILD" && ctest -C Debug 2>&1) |
    grep -E "\*\*\*(Failed|Not Run|Timeout)" |
    sed 's/.*Test *#[0-9]*: *//' | sed 's/[. ]*\*\*\*.*//' | sed 's#^#native #' > "$collected.native" || true
  compare "native tests" "$collected.native"
else
  echo "skipped: $MSIME_NATIVE_BUILD not configured"
fi

note "pipe-only tests"
if [ -d "$MSIME_PIPE_BUILD" ]; then
  ctest --test-dir "$MSIME_PIPE_BUILD" -C Debug 2>&1 |
    grep -E "\*\*\*(Failed|Not Run|Timeout)" |
    sed 's/.*Test *#[0-9]*: *//' | sed 's/[. ]*\*\*\*.*//' |
    sed 's#^#pipe #' > "$collected.pipe" || true
  compare "pipe-only tests" "$collected.pipe"
else
  echo "skipped: $MSIME_PIPE_BUILD not configured"
fi

if [ "$update" -eq 1 ]; then
  # Append-only: phases skip by host (wine), so what was not observed here is not evidence of a fix, and the comments beside each entry must stay where they are.
  added="$(cat "$collected".rust "$collected".native "$collected".pipe "$observed_wine" 2>/dev/null |
    grep -v '^ *$' | sort -u | comm -23 - <(grep -vE '^ *(#|$)' "$baseline" | sort -u) || true)"
  if [ -n "$added" ]; then
    { printf '\n# Added by --update-baseline on %s\n' "$(date +%F)"; printf '%s\n' "$added"; } >> "$baseline"
  fi
  echo
  echo "baseline: $(printf '%s' "$added" | grep -c . || true) new entries appended"
  exit 0
fi

echo
if [ "$failed" -eq 0 ]; then
  echo "verification passed: no failures outside the baseline"
else
  echo "verification FAILED"
fi
exit "$failed"
