# Third-party notices

## OpenCV

`crates/autosplit-core/src/imgproc.rs` and parts of `crates/autosplit-core/src/compare.rs` are
ports of OpenCV 4.11 routines (`resize`, `cvtColor`, `calcHist`, `normalize`, `compareHist`) and of
the `img_hash` module's pHash from opencv_contrib 4.11. They follow the original rounding so that
results match OpenCV exactly.

`crates/autosplit-capture/src/convert.rs` ports OpenCV's YUV to BGR conversions, and
`crates/autosplit-capture/src/dshow.rs` follows the DirectShow capture setup of OpenCV's
`videoio` module (`cap_dshow.cpp`), so frames match what AutoSplit gets from OpenCV.

OpenCV and opencv_contrib are licensed under the Apache License, Version 2.0.
Copyright (C) 2000-2022, Intel Corporation, all rights reserved.
Copyright (C) 2009-2011, Willow Garage Inc., all rights reserved.
Copyright (C) 2009-2016, NVIDIA Corporation, all rights reserved.
Copyright (C) 2010-2013, Advanced Micro Devices, Inc., all rights reserved.
Copyright (C) 2015-2023, OpenCV Foundation, all rights reserved.
Copyright (C) 2008-2016, Itseez Inc., all rights reserved.
Copyright (C) 2019-2023, Xperience AI, all rights reserved.
Copyright (C) 2019-2022, Shenzhen Institute of Artificial Intelligence and Robotics for Society, all rights reserved.
Copyright (C) 2022-2023, Southern University of Science and Technology, all rights reserved.

The license text is at <https://www.apache.org/licenses/LICENSE-2.0>.

## libjpeg-turbo

Linux builds include libjpeg-turbo 3.1, to decode MJPEG capture devices the way OpenCV does.
It's licensed under the Independent JPEG Group License, the Modified (3-clause) BSD License and
the zlib License. This software is based in part on the work of the Independent JPEG Group.
Copyright (C) 2009-2024 D. R. Commander. Copyright (C) 2015, 2020 Google, Inc.
Copyright (C) 1991-2020, Thomas G. Lane, Guido Vollbeding.

The license texts are at <https://github.com/libjpeg-turbo/libjpeg-turbo/blob/main/LICENSE.md>.

## Slint

The window is made with [Slint](https://slint.dev), used under the GNU General Public License
v3.0 (one of its license options). Copyright (C) SixtyFPS GmbH.

`vendor/i-slint-renderer-software` is Slint's CPU renderer with two changes for crisper text,
described in `vendor/README.md`, under the same license.

## AutoSplit

This project is a rewrite of [AutoSplit](https://github.com/Toufool/AutoSplit), licensed under
the GNU General Public License v3.0, which this project is also licensed under (see `LICENSE`).
The window's layout follows AutoSplit's.

`crates/autosplit-core/src/align.rs` ports AutoSplit's Align Region, which uses OpenCV's
`matchTemplate` (TM_SQDIFF); the matching is computed here with
[rustfft](https://github.com/ejmahler/RustFFT) (MIT or Apache-2.0).
