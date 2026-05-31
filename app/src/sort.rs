// Sorts by weight class always putting + class at very bottom
pub fn weight_class_key(class: &str) -> (i32, bool) {
    let weight = class.trim_end_matches("kg");
    let is_plus = weight.ends_with("+");
    let num_str = weight.trim_end_matches("+");
    let num = num_str.parse::<i32>().unwrap();

    (num, is_plus)
}

pub fn sort_by_class<T, F>(mut items: Vec<T>, mut get_weight_class: F) -> Vec<T>
where
    F: FnMut(&T) -> &str,
{
    items.sort_by_key(|item| weight_class_key(get_weight_class(item)));
    items
}
