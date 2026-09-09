//! 소소한 유틸리티.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// `f`를 워커 스레드에서 실행하고, `timeout` 안에 끝나면 그 결과를 `Some`으로,
/// 시간 안에 못 끝내면 `None`을 반환한다(워커는 방치한다).
///
/// macOS 개인정보 동의창(전체 디스크 접근)이 대기 중일 때 파일시스템 접근이
/// 무한 블록되는 것을 끊기 위한 용도. 반환하지 못한 워커 스레드는 프로세스가
/// 종료될 때까지 남지만, 단발 CLI 호출에서는 문제되지 않는다.
///
/// 주의: `f`는 반드시 **블록될 수 있는 작업**에만 써야 한다. 정당하게 오래
/// 걸리는 연산(예: userId SHA-512 역산)을 감싸면 잘못 끊긴다.
pub fn with_timeout<T, F>(timeout: Duration, f: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(timeout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_returns_fast_result() {
        let r = with_timeout(Duration::from_secs(2), || 21 * 2);
        assert_eq!(r, Some(42));
    }

    #[test]
    fn test_times_out_on_slow_work() {
        let r = with_timeout(Duration::from_millis(100), || {
            thread::sleep(Duration::from_secs(5));
            1
        });
        assert_eq!(r, None);
    }

    #[test]
    fn test_passes_through_result_type() {
        let r: Option<Result<i32, String>> =
            with_timeout(Duration::from_secs(1), || Ok(7));
        assert_eq!(r, Some(Ok(7)));
    }
}
