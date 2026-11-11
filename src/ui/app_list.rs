use super::{App, AppStatus, ReleaseCheck};
use craft_apps_manager::{catalog, model};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

#[derive(Default)]
pub(super) struct AppListCache(Option<(Inputs, Rc<AppList>)>);

struct Inputs {
    catalog_revision: u64,
    sidebar_order: Vec<String>,
    home_order: Vec<String>,
    release_format: String,
    hidden: Vec<String>,
    selected: Vec<String>,
    installed: Option<Vec<model::Installed>>,
    alternates: Vec<model::Installed>,
    checks: BTreeMap<String, (String, ReleaseCheck)>,
    known_updates: BTreeMap<(String, String, String), String>,
    checking: BTreeSet<String>,
}

impl Inputs {
    fn matches(&self, app: &App, revision: u64) -> bool {
        let prefs = &app.preferences;
        self.catalog_revision == revision
            && self.sidebar_order == prefs.app_order
            && self.home_order == prefs.home_app_order
            && self.release_format == prefs.release_format
            && self.hidden == prefs.hidden_apps
            && self.selected == prefs.selected_apps
            && self.installed.as_ref() == app.display_config.as_ref().map(|c| &c.apps)
            && self.alternates == app.alternates
            && self.checks == app.release_checks
            && self.known_updates == app.known_updates
            && self.checking == app.checking_apps
    }
}

#[derive(Default)]
pub(super) struct AppList {
    pub sidebar: Vec<String>,
    pub sidebar_visible: BTreeSet<String>,
    pub sidebar_updates: usize,
    pub installed: BTreeSet<String>,
    pub home_installed: Vec<String>,
    pub home_available: Vec<String>,
    pub home_installed_count: usize,
    pub home_updates: usize,
    pub home_checking: bool,
    pub home_checked: bool,
    pub chosen: usize,
    pub search_text: BTreeMap<String, String>,
    statuses: BTreeMap<String, Rc<AppStatus>>,
}

impl App {
    pub(super) fn app_list_snapshot(&mut self) -> Rc<AppList> {
        let revision = catalog::revision();
        if let Some((inputs, list)) = &self.app_list_cache.0 {
            if inputs.matches(self, revision) {
                return Rc::clone(list);
            }
        }

        let prefs = &self.preferences;
        let inputs = Inputs {
            catalog_revision: revision,
            sidebar_order: prefs.app_order.clone(),
            home_order: prefs.home_app_order.clone(),
            release_format: prefs.release_format.clone(),
            hidden: prefs.hidden_apps.clone(),
            selected: prefs.selected_apps.clone(),
            installed: self.display_config.as_ref().map(|c| c.apps.clone()),
            alternates: self.alternates.clone(),
            checks: self.release_checks.clone(),
            known_updates: self.known_updates.clone(),
            checking: self.checking_apps.clone(),
        };
        let mut list = AppList {
            sidebar: model::ordered_release_apps(&prefs.app_order, &prefs.release_format),
            ..Default::default()
        };
        for name in model::apps() {
            let status = self.status(&name);
            if status.installed.is_some() {
                list.installed.insert(name.clone());
            }
            list.search_text.insert(
                name.clone(),
                format!("{} {}", model::title(&name), model::category(&name)).to_lowercase(),
            );
            list.statuses.insert(name, Rc::new(status));
        }
        for name in &list.sidebar {
            if !prefs.hidden_apps.contains(name) {
                list.sidebar_visible.insert(name.clone());
            }
            list.sidebar_updates +=
                usize::from(list.statuses.get(name).is_some_and(|s| s.update.is_some()));
        }
        for name in model::ordered_release_apps(&prefs.home_app_order, &prefs.release_format) {
            let status = list.statuses.get(&name);
            let installed = status.is_some_and(|s| s.installed.is_some());
            list.home_installed_count += usize::from(installed);
            list.home_updates += usize::from(status.is_some_and(|s| s.update.is_some()));
            list.home_checking |= status.is_some_and(|s| s.checking);
            list.home_checked |= status.is_some_and(|s| s.check.is_some());
            if !prefs.hidden_apps.contains(&name) {
                if installed {
                    list.home_installed.push(name);
                } else {
                    list.home_available.push(name);
                }
            }
        }
        list.chosen = inputs.installed.as_ref().map_or(0, |apps| {
            apps.iter()
                .filter(|app| {
                    prefs.selected_apps.contains(&app.name)
                        && craft_apps_manager::updates::installed_for_updates(app, prefs)
                })
                .count()
        });
        let list = Rc::new(list);
        self.app_list_cache.0 = Some((inputs, Rc::clone(&list)));
        list
    }

    pub(super) fn app_list_status(&self, name: &str) -> Rc<AppStatus> {
        self.app_list_cache
            .0
            .as_ref()
            .and_then(|(_, list)| list.statuses.get(name))
            .cloned()
            .unwrap_or_else(|| Rc::new(self.status(name)))
    }
}
