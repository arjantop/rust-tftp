// Copyright 2014 Arjan Topolovec
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#[crate_id = "tftp"];
#[license = "MIT/ASL2"];
#[crate_type = "rlib"];
#[crate_type = "dylib"];
#[allow(deprecated_owned_vector)];
#[deny(warnings)];
#[feature(macro_rules, phase)];

extern crate collections;
extern crate rand;
#[phase(syntax, link)] extern crate log;

pub use common::TransferOptions;

pub mod protocol;

mod util;
mod common;
pub mod client;
