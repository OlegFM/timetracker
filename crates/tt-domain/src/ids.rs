/// Entity identifier (UUID v7).
pub type Id = uuid::Uuid;
/// Unix milliseconds, UTC.
pub type Millis = i64;

/// Creates a new time-sortable UUID v7.
pub fn new_id() -> Id {
    uuid::Uuid::now_v7()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_is_v7_and_sortable() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.get_version_num(), 7);
        assert_eq!(b.get_version_num(), 7);
        assert!(b > a);
    }
}
