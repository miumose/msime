#!/usr/bin/env python3
"""检查 Windows 原生输入习惯页没有遗漏共享偏好支持的候选项。"""

from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
source = (ROOT / "platforms/windows/settings/main.cpp").read_text(encoding="utf-8")
start = source.index("  void build_input_habits(")
end = source.index("\n    if (in_tab(tab, L\"output\"))", start)
page = source[start:end]

expected = {
    "只出单字": 'L"single_character_only"',
    "本地整句联想": 'L"sentence_association.word_lattice"',
}
for title, key in expected.items():
    assert title in page, f"Windows 输入习惯页缺少「{title}」"
    assert key in page, f"Windows 输入习惯页没有接入偏好键 {key}"

# 日文混输、桌面神经联想和资源包下载已随非拼音功能删除，这一页不应再列它们。
for removed in (
    "mixed_input.japanese",
    "build_neural_desktop_settings",
    "build_sentence_level_settings",
    "msime_client_resource_packs",
    'L"sentence_association.neural_keyboard"',
    'L"sentence_association.neural_desktop"',
):
    assert removed not in page, f"Windows 输入习惯页仍保留已删除功能 {removed}"

print("windows input habits: surviving candidate switches are present and removed features stay gone")
