$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Join-Path ([IO.Path]::GetTempPath()) ('msime-notices-' + [Guid]::NewGuid())
try {
    $prefix = Join-Path $root 'deps'
    New-Item -ItemType Directory -Force (Join-Path $prefix 'share/synthetic-lib') | Out-Null
    $license = Join-Path $prefix 'share/synthetic-lib/copyright'
    [IO.File]::WriteAllText($license, 'synthetic dependency copyright')
    $supplement = Join-Path $root 'extra.txt'
    [IO.File]::WriteAllText($supplement, 'synthetic extra notice')
    # Collect-Notices.ps1 读取随数据和代码一起提交的声明。
    $repositoryNotices = @('resources/licenses/msime-engine-dictionary-NOTICE.md', 'resources/helpcodes/ENGINE-NOTICE.md',
        'resources/helpcodes/NOTICE.md', 'resources/helpcodes/NOTICE-wubi86.md',
        'resources/licenses/pinyin-data-MIT.txt', 'resources/licenses/ECDICT-MIT.txt',
        'resources/licenses/rime_kaomoji_dict-MIT.txt', 'resources/licenses/python-pinyin-MIT.txt',
        'resources/licenses/rime-wanxiang-CC-BY-4.0.txt',
        'resources/licenses/sentence-model-Apache-2.0.txt',
        'resources/licenses/Administrative-divisions-of-China-WTFPL.txt',
        'resources/licenses/librime-octagram-BSD-3-Clause.txt',
        'resources/licenses/libcxx-Apache-2.0-WITH-LLVM-exception.txt',
        'resources/licenses/kfcd-chaizi-CC-BY-3.0.txt',
        'resources/licenses/Unicode-License-V3.txt',
        'platforms/windows/msimeui/UPSTREAM.md', 'platforms/windows/msimeui/LICENSE',
        'resources/licenses/OpenCC-Apache-2.0.txt', 'resources/licenses/onnxruntime-MIT.txt',
        'resources/licenses/onnxruntime-ThirdPartyNotices.txt', 'resources/licenses/rerank-model-Apache-2.0.txt',
        'resources/licenses/latex-symbols-MIT.txt')
    foreach ($relative in $repositoryNotices) {
        $path = Join-Path $root $relative
        New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
        [IO.File]::WriteAllText($path, "synthetic committed notice $relative")
    }
    # 版本表列出可以收集声明的产品版本。
    $editionTable = Join-Path $root 'shared/contracts/editions.json'
    New-Item -ItemType Directory -Force (Split-Path -Parent $editionTable) | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../../../../shared/contracts/editions.json') -Destination $editionTable
    $entry = Join-Path $PSScriptRoot '../../Collect-Notices.ps1'
    & $entry -RepoRoot $root -DependencyPrefixes @($prefix) -SupplementalNotices @($supplement)
    $output = Join-Path $root 'target/windows-notices/THIRD_PARTY_NOTICES.txt'
    $first = [IO.File]::ReadAllText($output)
    if (-not $first.Contains('synthetic extra notice') -or -not $first.Contains('synthetic dependency copyright') -or
        $first.Contains($root) -or $first.Contains('MSIME-Engine')) { throw 'Notice content/provenance mismatch' }
    foreach ($relative in $repositoryNotices) {
        if (-not $first.Contains("synthetic committed notice $relative") -or -not $first.Contains("($relative) =====")) {
            throw "Repository notice not collected: $relative"
        }
    }
    & $entry -RepoRoot $root -DependencyPrefixes @($prefix) -SupplementalNotices @($supplement)
    if ([IO.File]::ReadAllText($output) -ne $first) { throw 'Notice generation is not deterministic' }
    $dictionaryNotice = Join-Path $root 'resources/licenses/msime-engine-dictionary-NOTICE.md'
    # 每个版本收集的仓库声明都相同。
    $editionOutput = Join-Path $root 'edition-notices'
    & $entry -RepoRoot $root -DependencyPrefixes @($prefix) -SupplementalNotices @($supplement) -Edition pinyin -OutputDirectory $editionOutput
    $editionNotices = [IO.File]::ReadAllText((Join-Path $editionOutput 'THIRD_PARTY_NOTICES.txt'))
    foreach ($relative in $repositoryNotices) {
        if (-not $editionNotices.Contains("synthetic committed notice $relative")) { throw "Repository notice not collected for another edition: $relative" }
    }
    # An edition the table does not give a Windows section is refused.
    $rejected = $false
    try { & $entry -RepoRoot $root -DependencyPrefixes @($prefix) -Edition nosuchedition } catch { $rejected = $true }
    if (-not $rejected -or [IO.File]::ReadAllText($output) -ne $first) { throw 'Unknown edition accepted' }
    foreach ($failure in @('notice', 'license')) {
        # A missing repository notice is refused; the file is restored before the next case.
        if ($failure -eq 'notice') { Remove-Item -LiteralPath $dictionaryNotice }
        if ($failure -eq 'license') {
            [IO.File]::WriteAllText($dictionaryNotice, 'synthetic committed notice resources/licenses/msime-engine-dictionary-NOTICE.md')
            [IO.File]::WriteAllText($license, '')
        }
        $rejected = $false
        try { & $entry -RepoRoot $root -DependencyPrefixes @($prefix) } catch { $rejected = $true }
        if (-not $rejected -or [IO.File]::ReadAllText($output) -ne $first) { throw 'Failed collection damaged previous notices' }
    }
    Write-Output 'Notice provenance, deterministic output and failed-input preservation passed'
} finally {
    if (Test-Path $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}
