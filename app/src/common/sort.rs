/// Sort key for a weight class: numeric order, with the `+` class of a given
/// number after the bounded one, and classes with no number at all last.
///
/// Weight classes are scraped text (`"71kg"`, `"110+"`, `"102+kg"`), so a row
/// can carry something that holds no integer at all — an empty spreadsheet
/// cell, `"63.0"`, a stray label. Those have no numeric position, so they sort
/// after every numeric class. Parsing them was previously an `unwrap`, which
/// panicked the request instead of returning the rest of the table.
pub fn weight_class_key(class: &str) -> (bool, i32, bool) {
    let weight = class.trim_end_matches("kg");
    let is_plus = weight.ends_with("+");
    let num_str = weight.trim_end_matches("+");

    match num_str.parse::<i32>() {
        Ok(num) => (false, num, is_plus),
        Err(_) => (true, 0, is_plus),
    }
}

pub fn sort_by_class<T, F>(mut items: Vec<T>, mut get_weight_class: F) -> Vec<T>
where
    F: FnMut(&T) -> &str,
{
    items.sort_by_key(|item| weight_class_key(get_weight_class(item)));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(classes: &[&str]) -> Vec<String> {
        sort_by_class(
            classes.iter().map(|class| class.to_string()).collect(),
            |class| class.as_str(),
        )
    }

    #[test]
    fn orders_numeric_classes_with_plus_last() {
        assert_eq!(
            sorted(&["110+kg", "60kg", "110kg", "71kg"]),
            vec!["60kg", "71kg", "110kg", "110+kg"]
        );
        assert_eq!(sorted(&["110+", "60", "110"]), vec!["60", "110", "110+"]);
    }

    #[test]
    fn unparseable_classes_sort_last_instead_of_panicking() {
        // A blank sheet cell or a decimal weight used to panic the whole
        // request via `parse::<i32>().unwrap()`.
        assert_eq!(weight_class_key(""), (true, 0, false));
        assert_eq!(weight_class_key("63.0"), (true, 0, false));
        assert_eq!(weight_class_key("Youth"), (true, 0, false));
        assert_eq!(
            sorted(&["63.0", "110kg", "", "60kg"]),
            vec!["60kg", "110kg", "63.0", ""]
        );
    }
}
