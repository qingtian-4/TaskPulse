//! Bounded output and owned process trees for background native probes.
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
pub(crate) use windows::output;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::output;
