extern crate img_hash;
extern crate rayon;

use img_hash::demo::*;

use std::collections::VecDeque;

const HASH_WIDTH: u32 = 8;
const HASH_HEIGHT: u32 = 8;

// we perform the DCT on an enlarged image
const DCT_WIDTH: u32 = HASH_WIDTH * 2;
const DCT_HEIGHT: u32 = HASH_HEIGHT * 2;

macro_rules! handle(
    ($try:expr) => {
        if let Err(e) = $try {
            println!("{}", e);
        }
    }
);

fn main() -> Result<(), String> {
    let ref ctxt = DemoCtxt::init("demo_gradient", "HashAlg::Gradient")?;

    println!("generating DCT-mean hash demo, this will take some time");

    println!("generating grayscale animation");
    // 4 FPS over 5 seconds
    let grayscale_anim = ctxt.animate_grayscale(&ctxt.image, 20, 25);

    let ref grayscale = grayscale_anim.last().unwrap().buffer().clone();

    rayon::scope(move |s| {
        s.spawn(move |_| {
            println!("saving grayscale animation");
            handle!(ctxt.save_gif("grayscale", grayscale_anim));
            println!("done saving grayscale animation");
        });

        s.spawn(move |s| {
            println!("generating resize animation");
            let resize_anim = ctxt.animate_resize(grayscale, DCT_WIDTH, DCT_HEIGHT, 20, 25);

            s.spawn(move |_| {
                println!("saving resize animation");
                handle!(ctxt.save_gif("resize", resize_anim));
                println!("done saving resize animation");
            });
        });

        println!("generating DCT processing animation");
        let (dct_anim, dct) = animate_dct(ctxt, grayscale);

        s.spawn(move |_| {
            println!("saving DCT processing animation");
            handle!(ctxt.save_gif("dct", dct_anim));
            println!("done saving DCT processing animation");
        });

        println!("generating DCT crop animation");
        let (crop_anim, cropped) = animate_crop(ctxt, dct);

        s.spawn(move |_| {
            println!("saving DCT crop animation");
            handle!(ctxt.save_gif("dct_crop", crop_anim));
            println!("done saving DCT crop animation");
        });

        println!("generating mean animation");
        let (mean_anim, mean) = animate_mean(ctxt, cropped);

        s.spawn(move |_| {
            println!("saving DCT mean animation");
            handle!(ctxt.save_gif("dct_mean", mean_anim));
            println!("done saving DCT mean animation");
        });
    });

    Ok(())
}

/// A simple animation showing the DCT values sliding out of the original input
fn animate_dct(ctxt: &DemoCtxt, grayscale: &RgbaImage) -> (Vec<Frame>, RgbaImage) {
    let dct_ctxt = DctCtxt::new(DCT_WIDTH, DCT_HEIGHT);

    // the final resized image
    let resized_small = imageops::resize(grayscale, DCT_WIDTH, DCT_HEIGHT, Lanczos3);

    let input_len = resized_small.len() * 2;
    let mut vals_with_scratch = Vec::with_capacity(input_len);

    // put the image values in [..width * height] and provide scratch space
    vals_with_scratch.extend(resized_small.pixels().map(|px| px.to_luma()[0] as f32));
    // TODO: compare with `.set_len()`
    vals_with_scratch.resize(input_len, 0.);

    let dct_vals = dct_ctxt.dct_2d(vals_with_scratch);

    let mut dct_pxs = Vec::with_capacity(dct_vals.len() * 4);
    for val in dct_vals {
        dct_pxs.extend_from_slice(luma_rgba(val as u8).channels());
    }
    let dct_img = ImageBuffer::<Rgba<u8>, _>::from_vec(DCT_WIDTH, DCT_HEIGHT, dct_pxs).unwrap();

    let half_width = ctxt.width / 2;
    let gif_height = half_width;

    // 10% padding around the resized image
    let resize_width = fmul(half_width, 0.9);
    let resize_height = fmul(gif_height, 0.9);

    // center the input image in the left half of the gif
    let input_x = (half_width - resize_width) / 2;
    let input_y = (gif_height - resize_height) / 2;

    // center the output in the right half
    let output_x = input_x + half_width;
    let output_y = input_y;

    // as with `demo_gradient`, using Nearest gives sharp individual pixels
    let resized_input = imageops::resize(&resized_small, resize_width, resize_height, Nearest);
    let resized_output = imageops::resize(&dct_img, resize_width, resize_height, Nearest);

    let mut background = ImageBuffer::from_pixel(ctxt.width, gif_height, WHITE_A);
    imageops::overlay(&mut background, &resized_input, input_x, input_y);

    // first frame, just input, hold for 5 seconds
    let first_frame = Frame::from_parts(background.clone(), 0, 0, 500.into());

    let mut frames = Some(first_frame).into_iter().chain(
        lerp_iter(input_x, output_x, 5000, 24).map(|(x, frame_delay)| {
            let mut frame = background.clone();
            let mut output = resized_output.clone();

            if x < input_x + resize_width {
                // the part of the output image that overlaps the input image is inverted
                // and alpha set to one-half
                for (x, y) in x_y_iter(resize_width - (x - input_x), resize_height) {
                    let mut px = output.get_pixel_mut(x, y);
                    px.invert();
                    px[3] = 127;
                }
            }

            imageops::overlay(&mut frame, &output, x, output_y);
            Frame::from_parts(frame, 0, 0, frame_delay.into())
        })
    ).collect::<Vec<_>>();

    imageops::overlay(&mut background, &resized_output, output_x, output_y);

    frames.push(Frame::from_parts(background, 0, 0, 500.into()));

    (frames, dct_img)
}

fn animate_crop(ctxt: &DemoCtxt, mut full: RgbaImage) -> (Vec<Frame>, RgbaImage) {
    let cropped = full.sub_image(0, 0, HASH_WIDTH, HASH_HEIGHT).to_image();

    // don't assume a square hash in case we want to play with the values
    let gif_height = (ctxt.width * HASH_HEIGHT) / HASH_WIDTH;
    let full_width = fmul(ctxt.width, 0.8);
    let full_height = fmul(gif_height, 0.8);

    // extract the multiplier
    let cropped_width = (full_width * HASH_WIDTH) / DCT_WIDTH;
    let cropped_height = (full_height * HASH_HEIGHT) / DCT_HEIGHT;

    let background = rgba_fill_white(ctxt.width, gif_height);

    let resize_cropped = imageops::resize(&cropped, cropped_width, cropped_height, Nearest);
    let mut resize_full = imageops::resize(&full, full_width, full_height, Nearest);

    let (overlay_x, overlay_y) = center_at_point(ctxt.width / 2, gif_height / 2,
                                                 full_width, full_height);

    let mut frame = background.clone();
    imageops::overlay(&mut frame, &resize_full, overlay_x, overlay_y);

    // first frame, just original image for 2 seconds
    let first_frame = Frame::from_parts(frame.clone(), 0, 0, 200.into());

    let outline = Outline::new(cropped_width, cropped_height, fmul(cropped_width, 0.01));

    // second frame, draw crop outline
    outline.draw(&mut frame, overlay_x, overlay_y, RED);
    let second_frame = Frame::from_parts(frame, 0, 0, 200.into());

    for (x, y, mut px) in resize_full.enumerate_pixels_mut() {
        // the part of `full` that lies under `cropped` gets set to all zeros
        if x < cropped_width && y < cropped_height {
            px.data = [0; 4];
        } else {
            // the rest gets set to 70% alpha
            px[3] = fmul(255, 0.7) as u8;
        }
    }

    let remaining = resize_full;

    let mut background = ImageBuffer::from_pixel(ctxt.width, gif_height, WHITE_A);
    imageops::overlay(&mut background, &resize_cropped, overlay_x, overlay_y);
    outline.draw(&mut background, overlay_x, overlay_y, RED);

    let frames: Vec<_> = Some(first_frame).into_iter().chain(Some(second_frame)).chain(
        lerp_iter([overlay_x, overlay_y], [full_width, full_height], 2000, 10)
            .map(|([x, y], delay)| {
                let mut frame = background.clone();
                imageops::overlay(&mut frame, &remaining, x, y);

                // if `remaining` overlaps the outline, redraw the outline
                if x - overlay_x < outline.thickness || y - overlay_y < outline.thickness {
                    outline.draw(&mut frame, overlay_x, overlay_y, RED);
                }

                Frame::from_parts(frame, 0, 0, delay.into())
            })
    ).chain(
        // now resize the cropped part to full
        lerp_iter([cropped_width, cropped_height], [full_width, full_height], 2000, 10).map(
            |([w, h], delay)| {
                let mut frame = rgba_fill_white(ctxt.width, gif_height);
                let resized = imageops::resize(&cropped, w, h, Nearest);
                imageops::overlay(&mut frame, &resized, overlay_x, overlay_y);
                Frame::from_parts(frame, 0, 0, delay.into())
            }
        )
    ).collect();

    (frames, cropped)
}

/// framerate of the mean animation
const MEAN_FPS: u16 = 20;
const MEAN_FRAME_DELAY: u16 = 100 / MEAN_FPS;

fn animate_mean(ctxt: &DemoCtxt, data: RgbaImage) -> (Vec<Frame>, u8) {
    let resize_width = fmul(ctxt.width / 2, 0.9);
    let resize_height = resize_width * HASH_HEIGHT / HASH_WIDTH;

    let gif_height = ctxt.width / 2;

    let pixel_width = resize_width / HASH_WIDTH;
    let pixel_height = resize_height / HASH_HEIGHT;

    // required for `.sub_image()`
    let mut resized = imageops::resize(&data, resize_width, resize_height, Nearest);

    let outline_thickness = fmul(pixel_width, 0.1);
    let px_outline = Outline::new(pixel_width, pixel_height, outline_thickness);

    let mut mean = data.get_pixel(0, 0).to_luma()[0];

    let (resized_x, resized_y) = center_at_point(ctxt.width / 4, gif_height / 2,
                                                 resize_width, resize_height);

    let mut img_base = ImageBuffer::from_pixel(ctxt.width, gif_height, WHITE_A);
    imageops::overlay(&mut img_base, &resized, resized_x, resized_y);

    // intended effect: have each pixel fly on a curving path toward the mean pixel
    // support multiple pixels in-flight at once
    let mut inflight = VecDeque::new();
    let mut frames = vec![];

    // make the mean pixel twice as large to emphasize it
    let mean_width = pixel_width * 2;
    let mean_height = pixel_height * 2;

    // center the mean pixel in the right half of the frame
    let (target_x, target_y) = (ctxt.width * 3 / 4, gif_height / 2);

    let (mean_x, mean_y) = center_at_point(target_x, target_y, mean_width, mean_height);

    let (end_x, end_y) = center_at_point(target_x, target_y, pixel_width, pixel_height);

    // macros don't capture for their lifetime but they can access locals
    macro_rules! animate (
        () => {
            assert_ne!(inflight.len(), 0);

            let mut frame = img_base.clone();
            macro_rules! draw_mean(
                () => {
                    for (x, y) in x_y_iter(mean_width, mean_height) {
                        *frame.get_pixel_mut(mean_x + x, mean_y + y) = luma_rgba(mean);
                    }
                }
            );
            draw_mean!();

            // since we might remove indices while iterating, we'll need to visit them twice
            let mut i = 0;
            while i < inflight.len() {
                let (px_x, px_y) = inflight[i].1;

                let px = data.get_pixel(px_x, px_y).to_rgb();
                let mut outline_color = px.clone();
                outline_color.invert();

                if let Some(([next_x, next_y], _)) = inflight[i].0.next() {
                    let px_img = resized.sub_image(pixel_width * px_x, pixel_height * px_y,
                                               pixel_width, pixel_height);



                    overlay_generic(&mut frame, &px_img, next_x, next_y);
                    px_outline.draw(&mut frame, next_x, next_y, outline_color);
                    i += 1;
                } else {
                    mean = ((mean as u16 + px.to_luma()[0] as u16) / 2) as u8;
                    draw_mean!();
                    inflight.remove(i);
                    // `i` does not change as we have to revisit the index
                }
            }

            frames.push(Frame::from_parts(frame, 0, 0, MEAN_FRAME_DELAY.into()));
        }
    );

    for (x, y) in x_y_iter(HASH_WIDTH, HASH_HEIGHT) {
        let start = [resized_x + pixel_width * x, resized_y + pixel_width * y];

        let ctl1_x = resized_x + resize_width;
        // 10% over the middle of the image
        let ctl2_x = fmul(ctxt.width / 2, 1.1);

        let ctl_upper_y = gif_height;
        let ctl_lower_y = 0;

        let (ctl1, ctl2) = if y < gif_height / 2 {
            // go down then come back up
            ([ctl1_x, ctl_lower_y], [ctl2_x, ctl_upper_y])
        } else {
            // go up then come back down
            ([ctl1_x, ctl_upper_y], [ctl2_x, ctl_lower_y])
        };

        inflight.push_back((
            bez3_iter([start, ctl1, ctl2, [end_x, end_y]], 400, MEAN_FPS),
            (x, y)
        ));

        // run the animation for a couple frames before starting the next pixel
        for _ in 0 .. 2 {
            animate!();
        }
    }

    // finish the animations
    while !inflight.is_empty() {
        animate!();
    }

    (frames, mean)
}