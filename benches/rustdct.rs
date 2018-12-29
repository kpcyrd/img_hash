#[macro_use]
extern crate criterion;
extern crate rustdct;

// can't coerce Arc<Type2And3<f32>> to Arc<DCT2<f32>>
use rustdct::{DCTplanner, Type2And3 as DCT2};

use criterion::{Criterion, Fun};

use std::cmp;
use std::ops::{Deref, DerefMut};

struct StepIter {
    curr: usize,
    step: usize,
    max: usize,
}

impl Iterator for StepIter {
    type Item = usize;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let ret = self.curr;

        if ret >= self.max {
            None
        } else {
            self.curr += self.step;
            Some(ret)
        }
    }
}

fn transpose_safe(width: usize, height: usize, input: &[f32], output: &mut [f32]) {
    assert_eq!(input.len(), output.len());
    assert_eq!(width * height, input.len());
    assert_eq!(width * height, output.len());

    let output_len = output.len();

    let mut col_iter = (0 .. height).flat_map(|y| StepIter { curr: y, step: height, max: output_len});

    let mut chunks_exact = input.chunks_exact(16);
    for chunk in chunks_exact.by_ref() {
        for (val, out_idx) in chunk.iter().zip(&mut col_iter) {
            output[out_idx] = *val;
        }
    }

    for (val, out_idx) in chunks_exact.remainder().iter().zip(&mut col_iter) {
        output[out_idx] = *val;
    }
}

const BLOCK_SIZE: usize = 8;

#[inline(always)]
unsafe fn transpose_block<T: Copy>(input: &[T], output: &mut [T], width: usize, height: usize, block_x: usize, block_y: usize) {
    for inner_x in 0..BLOCK_SIZE {
        for inner_y in 0..BLOCK_SIZE {
            let x = block_x * BLOCK_SIZE + inner_x;
            let y = block_y * BLOCK_SIZE + inner_y;

            let input_index = x + y * width;
            let output_index = y + x * height;

            *output.get_unchecked_mut(output_index) = *input.get_unchecked(input_index);
        }
    }
}

#[inline(always)]
unsafe fn transpose_endcap_block<T: Copy>(input: &[T], output: &mut [T], width: usize, height: usize, block_x: usize, block_y: usize, block_width: usize, block_height: usize) {
    for inner_x in 0..block_width {
        for inner_y in 0..block_height {
            let x = block_x * BLOCK_SIZE + inner_x;
            let y = block_y * BLOCK_SIZE + inner_y;

            let input_index = x + y * width;
            let output_index = y + x * height;

            *output.get_unchecked_mut(output_index) = *input.get_unchecked(input_index);
        }
    }
}

/// Given an array of size width * height, representing a flattened 2D array,
/// transpose the rows and columns of that 2D array into the output
// Use "Loop tiling" to improve cache-friendliness
pub fn transpose_unsafe<T: Copy>(width: usize, height: usize, input: &[T], output: &mut [T]) {
    assert_eq!(width*height, input.len());
    assert_eq!(width*height, output.len());

    let x_block_count = width / BLOCK_SIZE;
    let y_block_count = height / BLOCK_SIZE;

    let remainder_x = width - x_block_count * BLOCK_SIZE;
    let remainder_y = height - y_block_count * BLOCK_SIZE;

    for y_block in 0..y_block_count {
        for x_block in 0..x_block_count {
            unsafe {
                transpose_block(
                    input, output,
                    width, height,
                    x_block, y_block);
            }
        }

        //if the width is not cleanly divisible by block_size, there are still a few columns that haven't been transposed
        if remainder_x > 0 {
            unsafe {
                transpose_endcap_block(
                    input, output,
                    width, height,
                    x_block_count, y_block,
                    remainder_x, BLOCK_SIZE);
            }
        }
    }

    //if the height is not cleanly divisible by BLOCK_SIZE, there are still a few columns that haven't been transposed
    if remainder_y > 0 {
        for x_block in 0..x_block_count {
            unsafe {
                transpose_endcap_block(
                    input, output,
                    width, height,
                    x_block, y_block_count,
                    BLOCK_SIZE, remainder_y,
                );
            }
        }

        //if the width is not cleanly divisible by block_size, there are still a few columns that haven't been transposed
        if remainder_x > 0 {
            unsafe {
                transpose_endcap_block(
                    input, output,
                    width, height,
                    x_block_count, y_block_count,
                    remainder_x, remainder_y);
            }
        }
    }
}

fn dct_2d<Ft>(row_dct: &DCT2<f32>, col_dct: &DCT2<f32>, signal: &mut [f32], width: usize, height: usize, transpose: Ft)
where Ft: Fn(usize, usize, &[f32], &mut [f32]) {
    assert_eq!(width * height, signal.len());
    let mut scratch = vec![0f32; signal.len()];

    for (row_in, row_out) in signal.chunks_mut(width).zip(scratch.chunks_mut(width)) {
        row_dct.process_dct2(row_in, row_out);
    }

    transpose(width, height, &scratch, signal);

    for (row_in, row_out) in signal.chunks_mut(height).zip(scratch.chunks_mut(height)) {
        col_dct.process_dct2(row_in, row_out);
    }

    transpose(width, height, &scratch, signal);
}

#[derive(Debug)]
struct AlignedVec {
    vec: Vec<f32>,
    start: usize,
    end: usize,
}

impl AlignedVec {
    pub fn new(len: usize) -> Self {
        let vec = vec![0.; len + 16];
        let start = (64 - vec.as_ptr() as usize % 64) / 4;
        AlignedVec {
            vec, start, end: start + len,
        }
    }

    pub fn unaligned(len: usize) -> Self {
        AlignedVec {
            vec: vec![0.; len], start: 0, end: len
        }
    }
}

impl Deref for AlignedVec {
    type Target = [f32];

    fn deref(&self) -> &[f32] {
        &self.vec[self.start .. self.end]
    }
}

impl DerefMut for AlignedVec {
    fn deref_mut(&mut self) -> &mut [f32] {
        &mut self.vec[self.start .. self.end]
    }
}

fn bench_transposes(criterion: &mut Criterion) {
    fn fill_data(width: usize, height: usize, out: &mut [f32]) {
        (0 .. height).flat_map(|y| (0 .. width).map(move |x| x as f32 * y as f32))
            .zip(out).for_each(|(val, out)| *out = val);
    }

    let dimensions = [
        [8, 8], [8, 16], [16, 8],
        [16, 16], [16, 32], [32, 16],
        [64, 64], [64, 128], [128, 64],
        [256, 256], [256, 512], [512, 256],
        [512, 512], [512, 1024], [1024, 512],
        [1024, 1024]
    ];


    for &[width, height] in &dimensions {
        let mut aligned = AlignedVec::new(width * height);
        fill_data(width, height, &mut aligned);

        let mut unaligned = AlignedVec::unaligned(width * height);
        fill_data(width, height, &mut unaligned);

        let fns = vec![
            Fun::new("Safe Transpose", move |b, data: &AlignedVec| {
                b.iter_with_setup(
                    || AlignedVec::new(width * height),
                    |mut out| {
                        transpose_safe(width, height, &data, &mut out);
                        out
                    }
                )
            }),
            Fun::new("Unsafe Transpose", move |b, data: &AlignedVec| {
                b.iter_with_setup(
                    || AlignedVec::new(width * height),
                    |mut out| {
                        transpose_unsafe(width, height, &data, &mut out);
                        out
                    }
                )
            })
        ];

        criterion.bench_functions(&format!("RustDCT {}x{} (aligned)", width, height), fns, aligned);

        let fns = vec![
            Fun::new("Safe Transpose", move |b, data: &AlignedVec| {
                b.iter_with_setup(
                    || AlignedVec::unaligned(width * height),
                    |mut out| {
                        transpose_safe(width, height, &data, &mut out);
                        out
                    }
                )
            }),
            Fun::new("Unsafe Transpose", move |b, data: &AlignedVec| {
                b.iter_with_setup(
                    || AlignedVec::unaligned(width * height),
                    |mut out| {
                        transpose_unsafe(width, height, &data, &mut out);
                        out
                    }
                )
            })
        ];

        criterion.bench_functions(&format!("RustDCT {}x{} (unaligned)", width, height), fns, unaligned);
    }
}

criterion_group!(benches, bench_transposes);
criterion_main!(benches);