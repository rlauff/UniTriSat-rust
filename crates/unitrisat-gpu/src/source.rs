use crate::Lane;

/// Emit subset-DP minors as scalar expressions. Each mask's determinant
/// uses the first popcount(mask) rows and the selected coordinate columns.
/// All masks precede their users, so no runtime allocation or recursion occurs.
pub(crate) fn generate(d: usize, lane: Lane) -> Option<String> {
    if !(2..=7).contains(&d) {
        return None;
    }
    let v = d.checked_add(1)?;
    let end = 1_usize.checked_shl(u32::try_from(d).ok()?)?;
    let mut s = format!(
        "#define D {d}\n#define V {v}\ntypedef {} T;\n",
        match lane {
            Lane::I32 => "int",
            Lane::I64 => "long long",
        }
    );
    s.push_str(
        "__device__ __forceinline__ void normal(const T e[D-1][D], T n[D]) {\nconst T m0 = 1;\n",
    );
    for mask in 1..end.checked_sub(1)? {
        let order = mask.count_ones();
        let row = order.checked_sub(1)?;
        let mut terms = Vec::new();
        let mut position = 0_u32;
        for col in 0..d {
            let bit = 1_usize.checked_shl(u32::try_from(col).ok()?)?;
            if mask & bit != 0 {
                let sign = if row.checked_add(position)?.checked_rem(2)? == 0 {
                    "+"
                } else {
                    "-"
                };
                terms.push(format!("{sign} e[{row}][{col}] * m{}", mask ^ bit));
                position = position.checked_add(1)?;
            }
        }
        s.push_str(&format!("const T m{mask} = {};\n", terms.join(" ")));
    }
    for col in 0..d {
        let mask = end.checked_sub(1)? ^ 1_usize.checked_shl(u32::try_from(col).ok()?)?;
        s.push_str(&format!(
            "n[{col}] = {}m{mask};\n",
            if col.checked_rem(2)? == 0 { "" } else { "-" }
        ));
    }
    s.push_str("}\n");
    // Each face row stores its vertices followed by the complement. The
    // zero-face family needs one representative (its span has no vectors).
    let mut offsets = Vec::new();
    let mut counts = Vec::new();
    let mut faces = Vec::new();
    let face_end = 1_usize.checked_shl(u32::try_from(v).ok()?)?;
    for k in 0..d {
        offsets.push(faces.len().checked_div(v)?);
        let mut count = 0_usize;
        for mask in 1..face_end {
            if mask.count_ones() != u32::try_from(k.checked_add(1)?).ok()? || (k == 0 && mask != 1)
            {
                continue;
            }
            for selected in [true, false] {
                for col in 0..v {
                    if (mask & 1_usize.checked_shl(u32::try_from(col).ok()?)? != 0) == selected {
                        faces.push(col.to_string());
                    }
                }
            }
            count = count.checked_add(1)?;
        }
        counts.push(count);
    }
    s.push_str(&format!(
        "__constant__ int faces[] = {{{}}};\n",
        faces.join(",")
    ));
    s.push_str(include_str!("kernel.cu"));
    // Facets first, then every mixed family (including both asymmetric ones).
    let top = d.checked_sub(1)?;
    let families = [top, 0].into_iter().chain(1..top);
    s.push_str("__device__ __forceinline__ bool pair(const T *a, const T *b) {\n");
    s.push_str("for (int c=0;c<D;++c) { T al=a[c],ah=al,bl=b[c],bh=bl; for(int r=1;r<V;++r) { al=min(al,a[r*D+c]); ah=max(ah,a[r*D+c]); bl=min(bl,b[r*D+c]); bh=max(bh,b[r*D+c]); } if(ah<=bl || bh<=al) return false; }\n");
    for p in families {
        let q = top.checked_sub(p)?;
        s.push_str(&format!("for (int ia=0;ia<{};++ia) for(int ib=0;ib<{};++ib) {{\nconst int *fa=faces+({}+ia)*V, *fb=faces+({}+ib)*V;\nT e[D-1][D],n[D];\n", counts.get(p)?,counts.get(q)?,offsets.get(p)?,offsets.get(q)?));
        s.push_str(&format!("for(int r=0;r<{p};++r) for(int c=0;c<D;++c) e[r][c]=a[fa[r+1]*D+c]-a[fa[0]*D+c];\nfor(int r=0;r<{q};++r) for(int c=0;c<D;++c) e[{p}+r][c]=b[fb[r+1]*D+c]-b[fb[0]*D+c];\nnormal(e,n); if(separated(a,b,n)) return false;\n}}\n"));
    }
    s.push_str("return true;\n}\n");
    s.push_str(include_str!("launch.cu"));
    Some(s)
}
