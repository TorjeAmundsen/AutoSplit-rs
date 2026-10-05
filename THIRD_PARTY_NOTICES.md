# Third-party notices

## OpenCV

`crates/autosplit-core/src/imgproc.rs` and parts of `crates/autosplit-core/src/compare.rs` are
ports of OpenCV 4.11 routines (`resize`, `cvtColor`, `calcHist`, `normalize`, `compareHist`) and of
the `img_hash` module's pHash from opencv_contrib 4.11. They follow the original rounding so that
results match OpenCV exactly.

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

## AutoSplit

This project is a rewrite of [AutoSplit](https://github.com/Toufool/AutoSplit), licensed under
the GNU General Public License v3.0, which this project is also licensed under (see `LICENSE`).
