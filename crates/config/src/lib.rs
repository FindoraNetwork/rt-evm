use {
    once_cell::sync::{Lazy, OnceCell},
    ruc::*,
    serde::{Deserialize, Serialize},
    std::{
        fs::File,
        io::{Read, Write},
        sync::atomic::AtomicU64,
    },
};
pub static CURRENT_BLOCK_HEIGHT: AtomicU64 = AtomicU64::new(0);

pub static CHECK_POINT_CONFIG_FILE_NAME: OnceCell<String> = OnceCell::new();

pub static CHECK_POINT_CONFIG: Lazy<CheckPointConfig> = Lazy::new(|| {
    let filename = pnk!(CHECK_POINT_CONFIG_FILE_NAME.get());
    pnk!(CheckPointConfig::load_from_file(filename))
});

#[derive(Debug, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct CheckPointConfig {
    pub min_gas_limit_v1_height: u64,
}
impl CheckPointConfig {
    pub fn load_from_file(filename: &str) -> Result<Self> {
        let mut file = File::open(filename).c(d!())?;

        let mut str = String::new();
        file.read_to_string(&mut str).c(d!())?;

        toml::from_str(&str).c(d!())
    }

    pub fn save_to_file(&self, filename: &str) -> Result<()> {
        let mut file = File::create(filename).c(d!())?;
        file.write_all(toml::to_string(self).c(d!())?.as_bytes())
            .c(d!())
    }
}
