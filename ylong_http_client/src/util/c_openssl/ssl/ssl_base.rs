// Copyright (c) 2023 Huawei Device Co., Ltd.
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use super::{error::HandshakeError, MidHandshakeSslStream, SslContext, SslErrorCode, SslStream};
use crate::{
    c_openssl::{
        check_ret,
        ffi::{
            bio::BIO,
            ssl::{
                SSL_connect, SSL_ctrl, SSL_get0_param, SSL_get_error, SSL_get_rbio,
                SSL_get_verify_result, SSL_read, SSL_state_string_long, SSL_write,
            },
        },
        foreign::ForeignRef,
        x509::{X509VerifyParamRef, X509VerifyResult, X509_CHECK_FLAG_NO_PARTIAL_WILDCARDS},
    },
    util::c_openssl::{
        check_ptr,
        error::ErrorStack,
        ffi::ssl::{SSL_free, SSL_new, SSL},
        foreign::Foreign,
    },
};
use core::{cmp, ffi, fmt, str};
use libc::{c_char, c_int, c_long, c_void};
use std::{
    ffi::CString,
    io::{Read, Write},
};

foreign_type!(
    type CStruct = SSL;
    fn drop = SSL_free;
    /// The main SSL/TLS structure.
    pub(crate) struct Ssl;
    pub(crate) struct SslRef;
);

impl Ssl {
    pub(crate) fn new(ctx: &SslContext) -> Result<Ssl, ErrorStack> {
        unsafe {
            let ptr = check_ptr(SSL_new(ctx.as_ptr()))?;
            Ok(Ssl::from_ptr(ptr))
        }
    }

    /// Client connect to Server.
    /// only `sync` use.
    #[cfg(feature = "sync")]
    pub(crate) fn connect<S>(self, stream: S) -> Result<SslStream<S>, HandshakeError<S>>
    where
        S: Read + Write,
    {
        let mut stream = SslStream::new_base(self, stream)?;
        let ret = unsafe { SSL_connect(stream.ssl.as_ptr()) };
        if ret > 0 {
            Ok(stream)
        } else {
            let error = stream.get_error(ret);
            match error.code {
                SslErrorCode::WANT_READ | SslErrorCode::WANT_WRITE => {
                    Err(HandshakeError::WouldBlock(MidHandshakeSslStream {
                        _stream: stream,
                        error,
                    }))
                }
                _ => Err(HandshakeError::Failure(MidHandshakeSslStream {
                    _stream: stream,
                    error,
                })),
            }
        }
    }
}

impl SslRef {
    pub(crate) fn get_error(&self, err: c_int) -> SslErrorCode {
        unsafe { SslErrorCode::from_int(SSL_get_error(self.as_ptr(), err)) }
    }

    fn ssl_status(&self) -> &'static str {
        let status = unsafe {
            let ptr = SSL_state_string_long(self.as_ptr());
            ffi::CStr::from_ptr(ptr as *const _)
        };
        str::from_utf8(status.to_bytes()).unwrap_or_default()
    }

    pub(crate) fn verify_result(&self) -> X509VerifyResult {
        unsafe { X509VerifyResult::from_raw(SSL_get_verify_result(self.as_ptr()) as c_int) }
    }

    pub(crate) fn get_raw_bio(&self) -> *mut BIO {
        unsafe { SSL_get_rbio(self.as_ptr()) }
    }

    pub(crate) fn read(&mut self, buf: &mut [u8]) -> c_int {
        let len = cmp::min(c_int::MAX as usize, buf.len()) as c_int;
        unsafe { SSL_read(self.as_ptr(), buf.as_ptr() as *mut c_void, len) }
    }

    pub(crate) fn write(&mut self, buf: &[u8]) -> c_int {
        let len = cmp::min(c_int::MAX as usize, buf.len()) as c_int;
        unsafe { SSL_write(self.as_ptr(), buf.as_ptr() as *const c_void, len) }
    }

    pub(crate) fn set_host_name(&mut self, name: &str) -> Result<(), ErrorStack> {
        let name = match CString::new(name) {
            Ok(name) => name,
            Err(_) => return Err(ErrorStack::get()),
        };
        check_ret(
            unsafe { ssl_set_tlsext_host_name(self.as_ptr(), name.as_ptr() as *mut _) } as c_int,
        )
        .map(|_| ())
    }

    pub(crate) fn param_mut(&mut self) -> &mut X509VerifyParamRef {
        unsafe { X509VerifyParamRef::from_ptr_mut(SSL_get0_param(self.as_ptr())) }
    }

    pub(crate) fn setup_verify_hostname(ssl: &mut SslRef, host: &str) -> Result<(), ErrorStack> {
        let param = ssl.param_mut();
        param.set_hostflags(X509_CHECK_FLAG_NO_PARTIAL_WILDCARDS);
        match host.parse() {
            Ok(ip) => param.set_ip(ip),
            Err(_) => param.set_host(host),
        }
    }
}

impl fmt::Debug for SslRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Ssl[state: {}, verify result: {}]",
            &self.ssl_status(),
            &self.verify_result()
        )
    }
}

const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 0x37;
const TLSEXT_NAMETYPE_HOST_NAME: c_int = 0x0;

unsafe fn ssl_set_tlsext_host_name(s: *mut SSL, name: *mut c_char) -> c_long {
    SSL_ctrl(
        s,
        SSL_CTRL_SET_TLSEXT_HOSTNAME,
        TLSEXT_NAMETYPE_HOST_NAME as c_long,
        name as *mut c_void,
    )
}
