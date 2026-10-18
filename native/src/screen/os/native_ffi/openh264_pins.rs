//! What this build loads of OpenH264 (decision C2 „OpenH264-Bezug“, NR N8):
//! version 2.6.0 from Cisco's server over HTTPS, the size and SHA-256 of the
//! compressed file and the SHA-256 of the unpacked library (the latter also
//! in the list of `openh264-sys2` 0.9.9), for 64-bit Windows and Linux on
//! x86-64, the targets of the release. Cisco publishes only MD5 sums; the
//! integrity rests on these pins alone, and they change only with a release.
//! The Cisco binary license v1.0 is shown before the download; its text is
//! kept word for word below (read on 2026-10-18 from
//! `https://www.openh264.org/BINARY_LICENSE.txt`, 4 612 bytes, SHA-256 in
//! `CISCO_BINARY_LICENSE_SHA256`). Rust reads `\r\n` in its source as `\n`, so
//! the text has the same bytes on every platform.

/// One download of the Cisco library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenH264Pin {
    pub url: &'static str,
    pub compressed_size: u64,
    pub compressed_sha256: &'static str,
    /// File name of the unpacked library in the store.
    pub library_name: &'static str,
    pub library_sha256: &'static str,
}

/// The pin of this platform; `None` where Cisco's file does not fit the
/// build (other architectures).
#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) const PIN: Option<OpenH264Pin> = Some(OpenH264Pin {
    url: "https://ciscobinary.openh264.org/openh264-2.6.0-win64.dll.bz2",
    compressed_size: 452_053,
    compressed_sha256: "dab5f2a872777f9a58b69bfa9fbcf20d9f82f2d6ec91383fd70bff49bd34ac9f",
    library_name: "openh264-2.6.0-win64.dll",
    library_sha256: "2076cb5675ec6c1a4c70e7a2a322552f547b6eeed649d6dfcd9e02a543b24691",
});
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) const PIN: Option<OpenH264Pin> = Some(OpenH264Pin {
    url: "https://ciscobinary.openh264.org/libopenh264-2.6.0-linux64.8.so.bz2",
    compressed_size: 634_264,
    compressed_sha256: "27ab53323c110b76214c1c72222f459d17febbcd1e252136cadc292b0308d75b",
    library_name: "libopenh264-2.6.0-linux64.8.so",
    library_sha256: "2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04",
});
#[cfg(not(any(
    all(windows, target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
pub(crate) const PIN: Option<OpenH264Pin> = None;

/// Host of every download; redirects are not followed.
pub(crate) const DOWNLOAD_HOST: &str = "https://ciscobinary.openh264.org/";
/// Largest unpacked library accepted (the 2.6.0 files are about 1 MB).
pub(crate) const MAX_LIBRARY_BYTES: u64 = 16 * 1024 * 1024;

/// SHA-256 of `CISCO_BINARY_LICENSE`.
pub(crate) const CISCO_BINARY_LICENSE_SHA256: &str =
    "bd9f363c5ea11ef723d0304cddacb5273c43c0e1194097c7a045d05273635418";

/// The Cisco binary license of OpenH264, v1.0, word for word.
pub(crate) const CISCO_BINARY_LICENSE: &str = r#"-------------------------------------------------------
About The Cisco-Provided Binary of OpenH264 Video Codec
-------------------------------------------------------

Cisco provides this program under the terms of the BSD license.  

Additionally, this binary is licensed under Cisco’s AVC/H.264 Patent Portfolio License from MPEG LA, at no cost to you, provided that the requirements and conditions shown below in the AVC/H.264 Patent Portfolio sections are met.  

As with all AVC/H.264 codecs, you may also obtain your own patent license from MPEG LA or from the individual patent owners, or proceed at your own risk.  Your rights from Cisco under the BSD license are not affected by this choice.  

For more information on the OpenH264 binary licensing, please see the OpenH264 FAQ found at http://www.openh264.org/faq.html#binary 

A corresponding source code to this binary program is available under the same BSD terms, which can be found at http://www.openh264.org

-----------
BSD License
-----------

Copyright © 2014 Cisco Systems, Inc.

All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS “AS IS” AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

-----------------------------------------
AVC/H.264 Patent Portfolio License Notice
-----------------------------------------

The binary form of this Software is distributed by Cisco under the AVC/H.264 Patent Portfolio License from MPEG LA, and is subject to the following requirements, which may or may not be applicable to your use of this software: 

THIS PRODUCT IS LICENSED UNDER THE AVC PATENT PORTFOLIO LICENSE FOR THE PERSONAL USE OF A CONSUMER OR OTHER USES IN WHICH IT DOES NOT RECEIVE REMUNERATION TO (i) ENCODE VIDEO IN COMPLIANCE WITH THE AVC STANDARD (“AVC VIDEO”) AND/OR (ii) DECODE AVC VIDEO THAT WAS ENCODED BY A CONSUMER ENGAGED IN A PERSONAL ACTIVITY AND/OR WAS OBTAINED FROM A VIDEO PROVIDER LICENSED TO PROVIDE AVC VIDEO.  NO LICENSE IS GRANTED OR SHALL BE IMPLIED FOR ANY OTHER USE.  ADDITIONAL INFORMATION MAY BE OBTAINED FROM MPEG LA, L.L.C. SEE HTTP://WWW.MPEGLA.COM

Accordingly, please be advised that content providers and broadcasters using AVC/H.264 in their service may be required to obtain a separate use license from MPEG LA, referred to as "(b) sublicenses" in the SUMMARY OF AVC/H.264 LICENSE TERMS from MPEG LA found at http://www.openh264.org/mpegla

---------------------------------------------
AVC/H.264 Patent Portfolio License Conditions
---------------------------------------------

In addition, the Cisco-provided binary of this Software is licensed under Cisco's license from MPEG LA only if the following conditions are met:

1. The Cisco-provided binary is separately downloaded to an end user’s device, and not integrated into or combined with third party software prior to being downloaded to the end user’s device;

2. The end user must have the ability to control (e.g., to enable, disable, or re-enable) the use of the Cisco-provided binary;

3. Third party software, in the location where end users can control the use of the Cisco-provided binary, must display the following text:

       "OpenH264 Video Codec provided by Cisco Systems, Inc."

4.  Any third-party software that makes use of the Cisco-provided binary must reproduce all of the above text, as well as this last condition, in the EULA and/or in another location where licensing information is to be presented to the end user.  
 


                          v1.0
"#;
