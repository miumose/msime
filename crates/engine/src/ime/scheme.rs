//! 当前输入方案。精简版只保留全拼和双拼。

use crate::quanpin::QuanpinScheme;
use crate::shuangpin::{ShuangpinProfile, ShuangpinScheme};
use crate::types::{QueryRequest, SchemeKey, SchemeType};

pub enum Scheme {
    Quanpin(QuanpinScheme),
    Shuangpin(ShuangpinScheme),
}

impl Scheme {
    pub fn new(scheme: SchemeType, profile: &'static ShuangpinProfile) -> Self {
        match scheme {
            SchemeType::Quanpin => Self::Quanpin(QuanpinScheme::new()),
            SchemeType::Shuangpin => Self::Shuangpin(ShuangpinScheme::new(profile)),
        }
    }

    pub fn scheme_type(&self) -> SchemeType {
        match self {
            Self::Quanpin(_) => SchemeType::Quanpin,
            Self::Shuangpin(_) => SchemeType::Shuangpin,
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Quanpin(scheme) => scheme.reset(),
            Self::Shuangpin(scheme) => scheme.reset(),
        }
    }

    pub fn handle_key(&mut self, key: SchemeKey) -> bool {
        match self {
            Self::Quanpin(scheme) => scheme.handle_key(key),
            Self::Shuangpin(scheme) => scheme.handle_key(key),
        }
    }

    pub fn build_request(&self) -> QueryRequest {
        match self {
            Self::Quanpin(scheme) => scheme.build_request(),
            Self::Shuangpin(scheme) => scheme.build_request(),
        }
    }

    pub fn build_request_into(&self, request: &mut QueryRequest) {
        match self {
            Self::Quanpin(scheme) => scheme.build_request_into(request),
            Self::Shuangpin(scheme) => scheme.build_request_into(request),
        }
    }

    #[allow(dead_code)]
    pub fn preedit(&self) -> String {
        match self {
            Self::Quanpin(scheme) => scheme.preedit(),
            Self::Shuangpin(scheme) => scheme.preedit(),
        }
    }

    pub fn set_raw_input(&mut self, raw: &str, raw_with_cases: &str) {
        match self {
            Self::Quanpin(scheme) => scheme.set_raw_input(raw, raw_with_cases),
            Self::Shuangpin(scheme) => scheme.set_raw_input(raw, raw_with_cases),
        }
    }

    pub fn as_shuangpin_mut(&mut self) -> Option<&mut ShuangpinScheme> {
        match self {
            Self::Shuangpin(scheme) => Some(scheme),
            _ => None,
        }
    }
}
