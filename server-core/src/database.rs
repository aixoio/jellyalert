use sqlx::SqlitePool;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub fn connect(database_path: &str) -> anyhow::Result<Self> {
        unimplemented!()
    }

    #[inline]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
