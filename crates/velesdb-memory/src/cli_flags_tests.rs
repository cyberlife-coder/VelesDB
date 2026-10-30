use super::{value_after, FlagError};

fn args(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn the_value_is_the_argument_after_the_flag() {
    let argv = args(&["--store", "/tmp/store", "--dry-run"]);

    assert_eq!(value_after(&argv, 0, "--store"), Ok(&argv[1]));
}

#[test]
fn a_flag_with_nothing_after_it_names_itself() {
    let argv = args(&["--dry-run", "--store"]);

    let refusal = value_after(&argv, 1, "--store").expect_err("nothing follows --store");

    assert_eq!(
        refusal,
        FlagError::MissingValue {
            flag: "--store".to_owned()
        }
    );
    assert_eq!(refusal.to_string(), "--store requires a value");
}

#[test]
fn an_unknown_flag_names_the_subcommand_and_the_flag() {
    let refusal = FlagError::Unknown {
        command: "compile-stdin",
        flag: "--nope".to_owned(),
    };

    assert_eq!(
        refusal.to_string(),
        r#"unknown compile-stdin flag "--nope""#
    );
}
