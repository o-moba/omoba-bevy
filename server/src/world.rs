//! Online host compatibility imports for the shared combat engine.
#[allow(unused_imports)]
pub(crate) use common::world::*;
use std::io;
pub(crate) fn load_map_config(
    path: Option<&std::path::Path>,
) -> io::Result<shared::map::ResolvedMap> {
    let Some(path) = path else {
        return Ok(shared::map::ResolvedMap::default());
    };
    use std::io::Read;
    let mut source = String::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(shared::map::MAX_CONFIG_BYTES as u64 + 1)
                .read_to_string(&mut source)
        })
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("OMOBA_MAP_CONFIG {}: {error}", path.display()),
            )
        })?;
    shared::map::MapDefinition::from_json(&source)
        .and_then(|map| map.resolve())
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("OMOBA_MAP_CONFIG {}: {error}", path.display()),
            )
        })
}
