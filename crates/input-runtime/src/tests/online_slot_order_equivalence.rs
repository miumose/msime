use super::*;

/// 对照的每页候选数：5，和偏好默认的 6。
const PAGE_SIZES: [u8; 2] = [5, 6];

// 参考实现按组依次追加下标，避免用生产代码的新座位偏移公式作为预期。AI 行不进分组（#6888）：其余行有云候选时按组排，没有时保持原顺序；第一条 AI 行再插进第一页最后一格（不足一页时接在其余行之后），其余 AI 行接在最后。
fn reference_order(
    sources: &[u8],
    positions: &[u8],
    single_kana: bool,
    page_size: u8,
) -> Vec<usize> {
    if sources.len() < 2 || !sources.iter().any(|source| matches!(source, 2 | 3 | 12)) {
        return (0..sources.len()).collect();
    }
    let group = |source| {
        sources
            .iter()
            .enumerate()
            .filter(|(_, value)| **value == source)
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    };
    let local: Vec<usize> = sources
        .iter()
        .enumerate()
        .filter(|(_, source)| !matches!(source, 2 | 3 | 4 | 6 | 7 | 12 | 13))
        .map(|(index, _)| index)
        .collect();
    let cloud = group(2);
    let ai: Vec<usize> = sources
        .iter()
        .enumerate()
        .filter(|(_, source)| matches!(source, 3 | 12))
        .map(|(index, _)| index)
        .collect();
    let mut order: Vec<usize> = if cloud.is_empty() {
        (0..sources.len())
            .filter(|index| !matches!(sources[*index], 3 | 12))
            .collect()
    } else {
        let english = group(4);
        let japanese = group(13);
        let emoji = group(6);
        let kaomoji = group(7);
        let promoted_english = !local.is_empty() && english.first() == Some(&0);
        let promoted_japanese =
            !local.is_empty() && japanese.first() == Some(&usize::from(promoted_english));
        let local_prefix = if single_kana { 2 } else { 1 };
        let mut order = Vec::new();
        if promoted_english {
            order.push(0);
        }
        if promoted_japanese {
            order.extend(japanese.first().copied());
        }
        order.extend(local.iter().take(local_prefix).copied());
        order.extend(&cloud);
        if !promoted_japanese {
            order.extend(japanese.first().copied());
        }
        if !promoted_english {
            order.extend(english.first().copied());
        }
        order.extend(emoji.first().copied());
        order.extend(kaomoji.first().copied());
        order.extend(local.iter().skip(local_prefix).copied());
        for group in [&english, &japanese, &emoji, &kaomoji] {
            order.extend(group.iter().skip(1).copied());
        }
        let mut fixed = Vec::new();
        order.retain(|&index| {
            if sources[index] == 4 && positions[index] > 0 {
                fixed.push(index);
                false
            } else {
                true
            }
        });
        fixed.sort_by_key(|&index| positions[index]);
        for index in fixed {
            order.insert(usize::from(positions[index] - 1).min(order.len()), index);
        }
        order
    };
    if let Some(&first) = ai.first() {
        let seat = usize::from(page_size - 1).max(1).min(order.len());
        order.insert(seat, first);
    }
    order.extend(ai.iter().skip(1).copied());
    order
}

