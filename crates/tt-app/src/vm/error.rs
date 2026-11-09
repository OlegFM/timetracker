//! Which `Tr` message to show for a service error, and a log code without user data.

use tt_services::ServiceError;

/// One `Tr.err-*` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorText {
    InUseProjects,
    InUseTasks,
    InUseEntries,
    Overlap,
    InvalidName,
    InvalidColor,
    InvalidRange,
    Backup,
    Generic,
}

pub fn error_text(e: &ServiceError) -> ErrorText {
    match e {
        ServiceError::InUse { what: "projects" } => ErrorText::InUseProjects,
        ServiceError::InUse { what: "tasks" } => ErrorText::InUseTasks,
        ServiceError::InUse { .. } => ErrorText::InUseEntries,
        ServiceError::Overlap => ErrorText::Overlap,
        ServiceError::Invalid("name") => ErrorText::InvalidName,
        ServiceError::Invalid("color") => ErrorText::InvalidColor,
        ServiceError::Invalid("range") => ErrorText::InvalidRange,
        ServiceError::Invalid("backup") => ErrorText::Backup,
        _ => ErrorText::Generic,
    }
}

/// Short fixed code for the log.
pub fn error_code(e: &ServiceError) -> &'static str {
    match e {
        ServiceError::Storage(_) => "storage",
        ServiceError::NotFound => "not found",
        ServiceError::InUse { what } => what,
        ServiceError::Overlap => "overlap",
        ServiceError::Invalid(what) => what,
        ServiceError::Io(_) => "io",
        ServiceError::Update(_) => "update",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_errors_and_falls_back_to_generic() {
        let cases = [
            (
                ServiceError::InUse { what: "projects" },
                ErrorText::InUseProjects,
            ),
            (ServiceError::InUse { what: "tasks" }, ErrorText::InUseTasks),
            (
                ServiceError::InUse {
                    what: "time_entries",
                },
                ErrorText::InUseEntries,
            ),
            (ServiceError::Overlap, ErrorText::Overlap),
            (ServiceError::Invalid("name"), ErrorText::InvalidName),
            (ServiceError::Invalid("color"), ErrorText::InvalidColor),
            (ServiceError::Invalid("range"), ErrorText::InvalidRange),
            (ServiceError::Invalid("backup"), ErrorText::Backup),
            (ServiceError::Invalid("not running"), ErrorText::Generic),
            (ServiceError::NotFound, ErrorText::Generic),
        ];
        for (e, want) in cases {
            assert_eq!(error_text(&e), want, "{e:?}");
        }
    }

    #[test]
    fn codes_carry_no_payload() {
        assert_eq!(error_code(&ServiceError::Update("secret".into())), "update");
        assert_eq!(error_code(&ServiceError::Invalid("name")), "name");
    }
}
