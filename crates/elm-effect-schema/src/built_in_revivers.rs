//! Built-in revivers: mapping from representation node kinds to the constructors
//! that recreate the corresponding schema values. The registry starts empty; as
//! concrete revivers land they are appended here rather than inlined so the
//! representation walk can stay generic.
pub fn built_in_revivers() -> Vec<&'static str> {
    vec![]
}
