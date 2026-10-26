//! Checked phase-case arithmetic for ownership plans.
use super::*;
pub(super) fn merge_contract(a: Option<Option<FrameContract>>, b: Option<Option<FrameContract>>)
    -> io::Result<Option<Option<FrameContract>>> {
    if a.is_some() && b.is_some() && a != b {return Err(invalid("capture plans have different output contracts"));}
    Ok(a.or(b))
}
pub(super) fn copy_requirements<const N: usize>(groups: [&[CaptureRequirement]; N]) -> io::Result<Vec<CaptureRequirement>> {
    let count = groups.iter().try_fold(0usize, |n, group| n.checked_add(group.len()))
        .ok_or_else(|| invalid("capture requirement count overflow"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(count).map_err(|_| io::Error::other("capture requirement allocation failed"))?;
    for item in groups.into_iter().flatten() {
        output.push(CaptureRequirement {object: CaptureObject(output.len()), source: item.source.clone(),
            generation: item.generation, owner: item.owner, bytes_each: item.bytes_each,
            count: item.count, phases: item.phases});
    }
    Ok(output)
}
pub(super) fn copy_cases(cases: &[CapturePhaseCase]) -> io::Result<Vec<CapturePhaseCase>> {
    let mut output = Vec::new();
    output.try_reserve_exact(cases.len()).map_err(|_| io::Error::other("capture phase table allocation failed"))?;
    output.extend_from_slice(cases);
    Ok(output)
}
pub(super) fn combine_cases(left: &[CapturePhaseCase], right: &[CapturePhaseCase]) -> io::Result<Vec<CapturePhaseCase>> {
    if right.is_empty() {return copy_cases(left);}
    let count = left.len().checked_mul(right.len()).ok_or_else(|| invalid("capture concurrency count overflow"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(count).map_err(|_| io::Error::other("capture concurrency table allocation failed"))?;
    for a in left {
        for b in right {
            let mut case = [[0; OWNER_COUNT]; PHASE_COUNT];
            for phase in 0..PHASE_COUNT {
                for owner in 0..OWNER_COUNT {
                    case[phase][owner] = a[phase][owner].checked_add(b[phase][owner])
                        .ok_or_else(|| invalid("capture concurrent owner overflow"))?;
                }
            }
            output.push(case);
        }
    }
    Ok(output)
}
pub(super) fn summarize(cases: &[CapturePhaseCase]) -> io::Result<(CapturePhaseCase, [u64; PHASE_COUNT])> {
    let mut table = [[0u64; OWNER_COUNT]; PHASE_COUNT];
    let mut totals = [0u64; PHASE_COUNT];
    for case in cases {
        for phase in 0..PHASE_COUNT {
            let mut total = 0u64;
            for owner in 0..OWNER_COUNT {
                table[phase][owner] = table[phase][owner].max(case[phase][owner]);
                total = total.checked_add(case[phase][owner]).ok_or_else(|| invalid("capture phase total overflow"))?;
            }
            totals[phase] = totals[phase].max(total);
        }
    }
    Ok((table, totals))
}
