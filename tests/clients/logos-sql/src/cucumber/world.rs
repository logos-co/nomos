use cucumber::World;
use logos_blockchain_tests::cucumber::world::CucumberWorld;
use logos_blockchain_zone_tests::cucumber::world::ZoneWorld;

use super::logos_sql::LogosSqlState;

#[derive(Debug, Default, World)]
pub struct SqlWorld {
    pub sdk: ZoneWorld,
    pub logos_sql: LogosSqlState,
}

impl SqlWorld {
    pub fn full_debug_info_string(&self) -> String {
        format!(
            "{}\n\nLogos SQL state:\n{:#?}",
            self.sdk.full_debug_info_string(),
            self.logos_sql,
        )
    }
}

impl AsMut<CucumberWorld> for SqlWorld {
    fn as_mut(&mut self) -> &mut CucumberWorld {
        &mut self.sdk.node
    }
}

impl AsMut<ZoneWorld> for SqlWorld {
    fn as_mut(&mut self) -> &mut ZoneWorld {
        &mut self.sdk
    }
}

impl Drop for SqlWorld {
    fn drop(&mut self) {
        // Stop SQL replicas before dropping their channel and node support.
        self.logos_sql.clear();
    }
}
