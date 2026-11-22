use super::{Rekor, bad};
use crate::Result;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) type Hash = [u8; 32];
pub(super) fn leaf(body: &[u8]) -> Hash {
    Sha256::new()
        .chain_update([0])
        .chain_update(body)
        .finalize()
        .into()
}
fn node(left: &Hash, right: &Hash) -> Hash {
    Sha256::new()
        .chain_update([1])
        .chain_update(left)
        .chain_update(right)
        .finalize()
        .into()
}
pub(super) fn inclusion(
    mut index: u64,
    size: u64,
    mut sum: Hash,
    path: &[Hash],
    root: Hash,
) -> Result<()> {
    if index >= size {
        return Err(bad("entry index outside tree"));
    }
    let mut last = size - 1;
    for sibling in path {
        if last == 0 {
            return Err(bad("excess inclusion proof"));
        }
        if index & 1 == 1 || index == last {
            sum = node(sibling, &sum);
            while index > 0 && index & 1 == 0 {
                index >>= 1;
                last >>= 1;
            }
        } else {
            sum = node(&sum, sibling);
        }
        index >>= 1;
        last >>= 1;
    }
    if last != 0 || sum != root {
        return Err(bad("invalid entry inclusion proof"));
    }
    Ok(())
}
pub(super) fn consistency(
    old: u64,
    new: u64,
    old_root: Hash,
    new_root: Hash,
    proof: &[Hash],
) -> Result<()> {
    if old == 0 || new <= old {
        return Err(bad("invalid consistency interval"));
    }
    let (mut left, mut right) = (old - 1, new - 1);
    while left & 1 == 1 {
        left >>= 1;
        right >>= 1;
    }
    let mut path = proof.iter();
    let mut first = if left == 0 {
        old_root
    } else {
        *path.next().ok_or_else(|| bad("missing consistency path"))?
    };
    let mut second = first;
    for sibling in path {
        if right == 0 {
            return Err(bad("excess consistency proof"));
        }
        if left & 1 == 1 || left == right {
            first = node(sibling, &first);
            second = node(sibling, &second);
            while left > 0 && left & 1 == 0 {
                left >>= 1;
                right >>= 1;
            }
        } else {
            second = node(&second, sibling);
        }
        left >>= 1;
        right >>= 1;
    }
    if right != 0 || first != old_root || second != new_root {
        return Err(bad("invalid tree consistency proof"));
    }
    Ok(())
}
fn index_path(mut n: u64) -> String {
    let mut parts = vec![format!("{:03}", n % 1000)];
    n /= 1000;
    while n > 0 {
        parts.push(format!("x{:03}", n % 1000));
        n /= 1000;
    }
    parts.reverse();
    parts.join("/")
}
// Tile leaves at tier t are complete subtrees of height 8*t. Use only
// logarithmically many aligned subtrees to authenticate both signed roots.
struct Tiles<'a> {
    log: &'a Rekor,
    size: u64,
    cache: BTreeMap<(u32, u64), Vec<u8>>,
}
impl Tiles<'_> {
    fn subtree(&mut self, start: u64, height: u32) -> Result<Hash> {
        let tier = height / 8;
        let offset = start >> (tier * 8);
        let number = offset / 256;
        let width = ((self.size >> (tier * 8)) - number * 256).min(256) as usize;
        if let std::collections::btree_map::Entry::Vacant(slot) = self.cache.entry((tier, number)) {
            let full = format!("/tile/{tier}/{}", index_path(number));
            let data = if width < 256 {
                let partial = format!("{full}.p/{width}");
                let r = self.log.get(&partial)?;
                if r.status() == 404 {
                    self.log.body(self.log.get(&full)?)?
                } else {
                    self.log.body(r)?
                }
            } else {
                self.log.body(self.log.get(&full)?)?
            };
            if data.len() != width * 32 && data.len() != 256 * 32 {
                return Err(bad("invalid tile length"));
            }
            slot.insert(data);
        }
        let tile = &self.cache[&(tier, number)];
        let start = (offset % 256) as usize;
        let len = 1usize << (height % 8);
        if start + len > width {
            return Err(bad("subtree outside authenticated tile"));
        }
        let mut hashes: Vec<Hash> = tile[start * 32..(start + len) * 32]
            .chunks_exact(32)
            .map(|v| v.try_into().unwrap())
            .collect();
        while hashes.len() > 1 {
            hashes = hashes.chunks_exact(2).map(|p| node(&p[0], &p[1])).collect();
        }
        Ok(hashes[0])
    }
    fn extend(&mut self, mut pos: u64, end: u64, stack: &mut Vec<(u32, Hash)>) -> Result<()> {
        while pos < end {
            let mut h = pos.trailing_zeros().min((end - pos).ilog2());
            let mut sum = self.subtree(pos, h)?;
            pos += 1u64 << h;
            while stack.last().is_some_and(|(level, _)| *level == h) {
                sum = node(&stack.pop().unwrap().1, &sum);
                h += 1;
            }
            stack.push((h, sum));
        }
        Ok(())
    }
}
fn fold(stack: &[(u32, Hash)]) -> Option<Hash> {
    stack
        .iter()
        .rev()
        .map(|v| v.1)
        .reduce(|right, left| node(&left, &right))
}
pub(super) fn tiled_consistency(
    log: &Rekor,
    old: u64,
    new: u64,
    old_root: Hash,
    new_root: Hash,
) -> Result<()> {
    let mut tiles = Tiles {
        log,
        size: new,
        cache: BTreeMap::new(),
    };
    let mut stack = Vec::new();
    tiles.extend(0, old, &mut stack)?;
    if fold(&stack) != Some(old_root) {
        return Err(bad("tiles differ from the earlier signed tree"));
    }
    tiles.extend(old, new, &mut stack)?;
    if fold(&stack) != Some(new_root) {
        return Err(bad("tiles differ from the current signed tree"));
    }
    Ok(())
}
