from pathlib import Path


def replace_exact(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    file_path.write_text(text.replace(old, new, 1), encoding="utf-8")


replace_exact(
    "native/src/engine/control.rs",
    """    pub(crate) fn sample_independent_observations<'a, T, F>(\n""",
    """    // The separate borrowed world/generation/buffer inputs keep this hot-path helper\n    // allocation-free and make each ownership boundary explicit.\n    #[allow(clippy::too_many_arguments)]\n    pub(crate) fn sample_independent_observations<'a, T, F>(\n""",
)

replace_exact(
    "native/src/engine/frame_v1.rs",
    """        for (index, (actual, expected)) in output\n            .chunks_exact(size_of::<f32>())\n            .zip(&fixture.expected.float_bits)\n            .enumerate()\n        {\n            let bytes: [u8; 4] = actual.try_into().expect(\"one complete Float32 word\");\n            assert_eq!(\n                u32::from_le_bytes(bytes),\n""",
    """        for (index, (actual, expected)) in output\n            .as_chunks::<{ size_of::<f32>() }>()\n            .0\n            .iter()\n            .zip(&fixture.expected.float_bits)\n            .enumerate()\n        {\n            assert_eq!(\n                u32::from_le_bytes(*actual),\n""",
)

replace_exact(
    "native/src/engine/inference_fixture.rs",
    """        for (index, (left_chunk, right_chunk)) in left\n            .as_bytes()\n            .chunks_exact(8)\n            .zip(right.as_bytes().chunks_exact(8))\n            .enumerate()\n""",
    """        for (index, (left_chunk, right_chunk)) in left\n            .as_bytes()\n            .as_chunks::<8>()\n            .0\n            .iter()\n            .zip(right.as_bytes().as_chunks::<8>().0.iter())\n            .enumerate()\n""",
)

replace_exact(
    "native/src/engine/state.rs",
    """        copies.truncate(0);\n""",
    """        copies.clear();\n""",
)

replace_exact(
    "native/src/engine/baseline_qualification.rs",
    """            let delivered_steps: Vec<_> = steps[next * 4..(next + rows) * 4]\n                .chunks_exact(4)\n                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))\n                .collect();\n""",
    """            let delivered_steps: Vec<_> = steps[next * 4..(next + rows) * 4]\n                .as_chunks::<4>()\n                .0\n                .iter()\n                .map(|bytes| u32::from_le_bytes(*bytes))\n                .collect();\n""",
)
