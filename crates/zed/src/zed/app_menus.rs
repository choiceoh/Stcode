use gpui::{App, Menu, MenuItem, OsAction};
use release_channel::ReleaseChannel;
use terminal_view::terminal_panel;
use zed_actions::{debug_panel, dev};

pub fn app_menus(cx: &mut App) -> Vec<Menu> {
    let app_name = super::app_display_name(cx);
    let is_stcode = workspace::AppLaunchMode::is_stcode(cx);

    if is_stcode {
        stcode_app_menus(cx, app_name, stcode_help_items())
    } else {
        zed_app_menus(cx, app_name, zed_help_items())
    }
}

fn zoom_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action(
            "Zoom In",
            zed_actions::IncreaseBufferFontSize { persist: false },
        ),
        MenuItem::action(
            "Zoom Out",
            zed_actions::DecreaseBufferFontSize { persist: false },
        ),
        MenuItem::action(
            "Reset Zoom",
            zed_actions::ResetBufferFontSize { persist: false },
        ),
        MenuItem::action(
            "Reset All Zoom",
            zed_actions::ResetAllZoom { persist: false },
        ),
    ]
}

fn zed_view_items(cx: &mut App) -> Vec<MenuItem> {
    let mut view_items = zoom_items();
    view_items.extend([
        MenuItem::separator(),
        MenuItem::action("Toggle Left Dock", workspace::ToggleLeftDock),
        MenuItem::action("Toggle Right Dock", workspace::ToggleRightDock),
        MenuItem::action("Toggle Bottom Dock", workspace::ToggleBottomDock),
        MenuItem::action("Toggle All Docks", workspace::ToggleAllDocks),
        MenuItem::submenu(Menu {
            name: "Editor Layout".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Split Up", workspace::SplitUp::default()),
                MenuItem::action("Split Down", workspace::SplitDown::default()),
                MenuItem::action("Split Left", workspace::SplitLeft::default()),
                MenuItem::action("Split Right", workspace::SplitRight::default()),
            ],
        }),
        MenuItem::separator(),
        MenuItem::action("Project Panel", zed_actions::project_panel::ToggleFocus),
        MenuItem::action("Outline Panel", outline_panel::ToggleFocus),
        MenuItem::action("Terminal Panel", terminal_panel::ToggleFocus),
        MenuItem::action("Debugger Panel", debug_panel::ToggleFocus),
        MenuItem::separator(),
        MenuItem::action("Diagnostics", diagnostics::Deploy),
        MenuItem::separator(),
    ]);

    if ReleaseChannel::try_global(cx) == Some(ReleaseChannel::Dev) {
        view_items.push(MenuItem::action(
            "Toggle GPUI Inspector",
            dev::ToggleInspector,
        ));
        view_items.push(MenuItem::separator());
    }

    view_items
}

fn stcode_view_items(cx: &mut App) -> Vec<MenuItem> {
    let mut view_items = zoom_items();
    view_items.extend([
        MenuItem::separator(),
        MenuItem::action("Focus Agent", zed_actions::assistant::FocusAgent),
        MenuItem::action("Toggle Agent Panel", zed_actions::assistant::ToggleFocus),
    ]);

    if ReleaseChannel::try_global(cx) == Some(ReleaseChannel::Dev) {
        view_items.push(MenuItem::separator());
        view_items.push(MenuItem::action(
            "Toggle GPUI Inspector",
            dev::ToggleInspector,
        ));
    }

    view_items
}

fn zed_help_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action(
            "View Release Notes Locally",
            auto_update_ui::ViewReleaseNotesLocally,
        ),
        MenuItem::action("View Telemetry", zed_actions::OpenTelemetryLog),
        MenuItem::action("View Dependency Licenses", zed_actions::OpenLicenses),
        MenuItem::action("Show Welcome", onboarding::ShowWelcome),
        MenuItem::separator(),
        MenuItem::action(
            "Documentation",
            super::OpenBrowser {
                url: "https://zed.dev/docs".into(),
            },
        ),
        MenuItem::action(
            "Zed Twitter",
            super::OpenBrowser {
                url: "https://twitter.com/zeddotdev".into(),
            },
        ),
        MenuItem::action(
            "Join the Team",
            super::OpenBrowser {
                url: "https://zed.dev/jobs".into(),
            },
        ),
    ]
}

fn stcode_help_items() -> Vec<MenuItem> {
    vec![MenuItem::action(
        "View Dependency Licenses",
        zed_actions::OpenLicenses,
    )]
}

fn zed_app_menus(cx: &mut App, app_name: &'static str, help_items: Vec<MenuItem>) -> Vec<Menu> {
    vec![
        zed_app_menu(app_name),
        zed_file_menu(),
        zed_edit_menu(),
        zed_selection_menu(),
        Menu {
            name: "View".into(),
            disabled: false,
            items: zed_view_items(cx),
        },
        zed_go_menu(),
        zed_run_menu(),
        window_menu(),
        Menu {
            name: "Help".into(),
            disabled: false,
            items: help_items,
        },
    ]
}

fn stcode_app_menus(cx: &mut App, app_name: &'static str, help_items: Vec<MenuItem>) -> Vec<Menu> {
    vec![
        stcode_app_menu(app_name),
        stcode_file_menu(),
        stcode_agent_menu(),
        stcode_edit_menu(),
        Menu {
            name: "View".into(),
            disabled: false,
            items: stcode_view_items(cx),
        },
        window_menu(),
        Menu {
            name: "Help".into(),
            disabled: false,
            items: help_items,
        },
    ]
}

fn zed_app_menu(app_name: &'static str) -> Menu {
    Menu {
        name: app_name.into(),
        disabled: false,
        items: vec![
            MenuItem::action(format!("About {app_name}"), zed_actions::About),
            MenuItem::action("Check for Updates", auto_update::Check),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Settings").items([
                MenuItem::action("Open Settings", zed_actions::OpenSettings),
                MenuItem::action("Open Settings File", super::OpenSettingsFile),
                MenuItem::action("Open Project Settings", zed_actions::OpenProjectSettings),
                MenuItem::action("Open Project Settings File", super::OpenProjectSettingsFile),
                MenuItem::action("Open Default Settings", super::OpenDefaultSettings),
                MenuItem::separator(),
                MenuItem::action("Open Keymap File", zed_actions::OpenKeymapFile),
                MenuItem::action("Open Default Key Bindings", zed_actions::OpenDefaultKeymap),
                MenuItem::separator(),
                MenuItem::action(
                    "Select Theme...",
                    zed_actions::theme_selector::Toggle::default(),
                ),
                MenuItem::action(
                    "Select Icon Theme...",
                    zed_actions::icon_theme_selector::Toggle::default(),
                ),
            ])),
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Extensions", zed_actions::Extensions::default()),
            #[cfg(not(target_os = "windows"))]
            MenuItem::action("Install CLI", install_cli::InstallCliBinary),
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::action(format!("Hide {app_name}"), super::Hide),
            #[cfg(target_os = "macos")]
            MenuItem::action("Hide Others", super::HideOthers),
            #[cfg(target_os = "macos")]
            MenuItem::action("Show All", super::ShowAll),
            MenuItem::separator(),
            MenuItem::action(format!("Quit {app_name}"), zed_actions::Quit),
        ],
    }
}

fn stcode_app_menu(app_name: &'static str) -> Menu {
    Menu {
        name: app_name.into(),
        disabled: false,
        items: vec![
            MenuItem::action(format!("About {app_name}"), zed_actions::About),
            MenuItem::action("Check for Updates", auto_update::Check),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Settings").items([
                MenuItem::action("Open Settings", zed_actions::OpenSettings),
                MenuItem::action("Open Settings File", super::OpenSettingsFile),
                MenuItem::action("Open Workspace Settings", zed_actions::OpenProjectSettings),
                MenuItem::action(
                    "Open Workspace Settings File",
                    super::OpenProjectSettingsFile,
                ),
                MenuItem::separator(),
                MenuItem::action("Open Default Settings", super::OpenDefaultSettings),
                MenuItem::separator(),
                MenuItem::action("Open Default Key Bindings", zed_actions::OpenDefaultKeymap),
                MenuItem::separator(),
                MenuItem::action(
                    "Select Theme...",
                    zed_actions::theme_selector::Toggle::default(),
                ),
                MenuItem::action(
                    "Select Icon Theme...",
                    zed_actions::icon_theme_selector::Toggle::default(),
                ),
            ])),
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::action(format!("Hide {app_name}"), super::Hide),
            #[cfg(target_os = "macos")]
            MenuItem::action("Hide Others", super::HideOthers),
            #[cfg(target_os = "macos")]
            MenuItem::action("Show All", super::ShowAll),
            MenuItem::separator(),
            MenuItem::action(format!("Quit {app_name}"), zed_actions::Quit),
        ],
    }
}

fn zed_file_menu() -> Menu {
    Menu {
        name: "File".into(),
        disabled: false,
        items: vec![
            MenuItem::action("New", workspace::NewFile),
            MenuItem::action("New Window", workspace::NewWindow),
            MenuItem::separator(),
            #[cfg(not(target_os = "macos"))]
            MenuItem::action("Open File...", workspace::OpenFiles),
            MenuItem::action(
                if cfg!(not(target_os = "macos")) {
                    "Open Folder..."
                } else {
                    "Open…"
                },
                workspace::Open::default(),
            ),
            MenuItem::action(
                "Open Recent...",
                zed_actions::OpenRecent {
                    create_new_window: false,
                },
            ),
            MenuItem::action(
                "Open Remote...",
                zed_actions::OpenRemote {
                    create_new_window: false,
                    from_existing_connection: false,
                },
            ),
            MenuItem::separator(),
            MenuItem::action("Add Folder to Project…", workspace::AddFolderToProject),
            MenuItem::separator(),
            MenuItem::action("Save", workspace::Save { save_intent: None }),
            MenuItem::action("Save As…", workspace::SaveAs),
            MenuItem::action("Save All", workspace::SaveAll { save_intent: None }),
            MenuItem::separator(),
            MenuItem::action(
                "Close Editor",
                workspace::CloseActiveItem {
                    save_intent: None,
                    close_pinned: true,
                },
            ),
            MenuItem::action("Close Project", workspace::CloseProject),
            MenuItem::action("Close Window", workspace::CloseWindow),
        ],
    }
}

fn stcode_file_menu() -> Menu {
    Menu {
        name: "File".into(),
        disabled: false,
        items: vec![
            MenuItem::action("New Window", workspace::NewWindow),
            MenuItem::separator(),
            #[cfg(not(target_os = "macos"))]
            MenuItem::action("Open File...", workspace::OpenFiles),
            MenuItem::action("Open Workspace...", workspace::Open::default()),
            MenuItem::action(
                "Open Recent Workspaces...",
                zed_actions::OpenRecent {
                    create_new_window: false,
                },
            ),
            MenuItem::action(
                "Open Remote Workspace...",
                zed_actions::OpenRemote {
                    create_new_window: false,
                    from_existing_connection: false,
                },
            ),
            MenuItem::separator(),
            MenuItem::action("Add Folder to Workspace…", workspace::AddFolderToProject),
            MenuItem::separator(),
            MenuItem::action("Save", workspace::Save { save_intent: None }),
            MenuItem::action("Save As…", workspace::SaveAs),
            MenuItem::action("Save All", workspace::SaveAll { save_intent: None }),
            MenuItem::separator(),
            MenuItem::action("Close Workspace", workspace::CloseProject),
            MenuItem::action("Close Window", workspace::CloseWindow),
        ],
    }
}

fn zed_edit_menu() -> Menu {
    Menu {
        name: "Edit".into(),
        disabled: false,
        items: vec![
            MenuItem::os_action("Undo", editor::actions::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", editor::actions::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", editor::actions::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", editor::actions::Copy, OsAction::Copy),
            MenuItem::action("Copy and Trim", editor::actions::CopyAndTrim),
            MenuItem::os_action("Paste", editor::actions::Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::action("Find", search::buffer_search::Deploy::find()),
            MenuItem::action("Find in Project", workspace::DeploySearch::default()),
            MenuItem::separator(),
            MenuItem::action(
                "Toggle Line Comment",
                editor::actions::ToggleComments::default(),
            ),
        ],
    }
}

fn stcode_edit_menu() -> Menu {
    Menu {
        name: "Edit".into(),
        disabled: false,
        items: vec![
            MenuItem::os_action("Undo", editor::actions::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", editor::actions::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", editor::actions::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", editor::actions::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", editor::actions::Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::action("Find", search::buffer_search::Deploy::find()),
            MenuItem::action("Find in Workspace", workspace::DeploySearch::default()),
            MenuItem::separator(),
            MenuItem::os_action(
                "Select All",
                editor::actions::SelectAll,
                OsAction::SelectAll,
            ),
        ],
    }
}

fn stcode_agent_menu() -> Menu {
    Menu {
        name: "Agent".into(),
        disabled: false,
        items: vec![
            MenuItem::action("New Thread", agent_ui::NewThread),
            MenuItem::action("New Thread Menu...", agent_ui::ToggleNewThreadMenu),
            MenuItem::separator(),
            MenuItem::action("Focus Agent", zed_actions::assistant::FocusAgent),
            MenuItem::action("Toggle Agent Panel", zed_actions::assistant::ToggleFocus),
            MenuItem::separator(),
            MenuItem::action(
                "Open Rules Library",
                zed_actions::assistant::OpenRulesLibrary::default(),
            ),
            MenuItem::action(
                "Open Active Thread as Markdown",
                agent_ui::OpenActiveThreadAsMarkdown,
            ),
            MenuItem::action("Open Agent Diff", agent_ui::OpenAgentDiff),
            MenuItem::separator(),
            MenuItem::action("Command Palette...", zed_actions::command_palette::Toggle),
        ],
    }
}

fn zed_selection_menu() -> Menu {
    Menu {
        name: "Selection".into(),
        disabled: false,
        items: vec![
            MenuItem::os_action(
                "Select All",
                editor::actions::SelectAll,
                OsAction::SelectAll,
            ),
            MenuItem::action("Expand Selection", editor::actions::SelectLargerSyntaxNode),
            MenuItem::action("Shrink Selection", editor::actions::SelectSmallerSyntaxNode),
            MenuItem::action("Select Next Sibling", editor::actions::SelectNextSyntaxNode),
            MenuItem::action(
                "Select Previous Sibling",
                editor::actions::SelectPreviousSyntaxNode,
            ),
            MenuItem::separator(),
            MenuItem::action(
                "Add Cursor Above",
                editor::actions::AddSelectionAbove {
                    skip_soft_wrap: true,
                },
            ),
            MenuItem::action(
                "Add Cursor Below",
                editor::actions::AddSelectionBelow {
                    skip_soft_wrap: true,
                },
            ),
            MenuItem::action(
                "Select Next Occurrence",
                editor::actions::SelectNext {
                    replace_newest: false,
                },
            ),
            MenuItem::action(
                "Select Previous Occurrence",
                editor::actions::SelectPrevious {
                    replace_newest: false,
                },
            ),
            MenuItem::action("Select All Occurrences", editor::actions::SelectAllMatches),
            MenuItem::separator(),
            MenuItem::action("Move Line Up", editor::actions::MoveLineUp),
            MenuItem::action("Move Line Down", editor::actions::MoveLineDown),
            MenuItem::action("Duplicate Selection", editor::actions::DuplicateLineDown),
        ],
    }
}

fn zed_go_menu() -> Menu {
    Menu {
        name: "Go".into(),
        disabled: false,
        items: vec![
            MenuItem::action("Back", workspace::GoBack),
            MenuItem::action("Forward", workspace::GoForward),
            MenuItem::separator(),
            MenuItem::action("Command Palette...", zed_actions::command_palette::Toggle),
            MenuItem::separator(),
            MenuItem::action("Go to File...", workspace::ToggleFileFinder::default()),
            // MenuItem::action("Go to Symbol in Project", project_symbols::Toggle),
            MenuItem::action(
                "Go to Symbol in Editor...",
                zed_actions::outline::ToggleOutline,
            ),
            MenuItem::separator(),
            MenuItem::action("Go to Definition", editor::actions::GoToDefinition),
            MenuItem::action("Go to Declaration", editor::actions::GoToDeclaration),
            MenuItem::action("Go to Type Definition", editor::actions::GoToTypeDefinition),
            MenuItem::action(
                "Find All References",
                editor::actions::FindAllReferences::default(),
            ),
            MenuItem::separator(),
            MenuItem::action("Next Problem", editor::actions::GoToDiagnostic::default()),
            MenuItem::action(
                "Previous Problem",
                editor::actions::GoToPreviousDiagnostic::default(),
            ),
        ],
    }
}

fn zed_run_menu() -> Menu {
    Menu {
        name: "Run".into(),
        disabled: false,
        items: vec![
            MenuItem::action(
                "Spawn Task",
                zed_actions::Spawn::ViaModal {
                    reveal_target: None,
                },
            ),
            MenuItem::action("Edit tasks.json...", crate::zed::OpenProjectTasks),
        ],
    }
}

fn window_menu() -> Menu {
    Menu {
        name: "Window".into(),
        disabled: false,
        items: vec![
            MenuItem::action("Minimize", super::Minimize),
            MenuItem::action("Zoom", super::Zoom),
            MenuItem::separator(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Menu, MenuItem};

    fn action_names(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect()
    }

    fn submenu<'a>(menu: &'a Menu, name: &str) -> &'a Menu {
        menu.items
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu(submenu) if submenu.name.as_ref() == name => Some(submenu),
                _ => None,
            })
            .expect("submenu should exist")
    }

    #[test]
    fn test_stcode_file_menu_uses_workspace_terms() {
        let file_menu = super::stcode_file_menu();
        let labels = action_names(&file_menu);

        assert!(labels.contains(&"Open Workspace...".to_string()));
        assert!(labels.contains(&"Open Recent Workspaces...".to_string()));
        assert!(labels.contains(&"Open Remote Workspace...".to_string()));
        assert!(labels.contains(&"Add Folder to Workspace…".to_string()));
        assert!(labels.contains(&"Close Workspace".to_string()));

        assert!(!labels.contains(&"Open Recent...".to_string()));
        assert!(!labels.contains(&"Open Remote...".to_string()));
        assert!(!labels.contains(&"Close Project".to_string()));
    }

    #[test]
    fn test_stcode_settings_menu_keeps_workspace_settings() {
        let app_menu = super::stcode_app_menu("Stcode");
        let settings_menu = submenu(&app_menu, "Settings");
        let labels = action_names(settings_menu);

        assert!(labels.contains(&"Open Workspace Settings".to_string()));
        assert!(labels.contains(&"Open Workspace Settings File".to_string()));
        assert!(!labels.contains(&"Open Project Settings".to_string()));
        assert!(!labels.contains(&"Open Project Settings File".to_string()));
    }

    #[test]
    fn test_stcode_help_menu_removes_zed_support_surfaces() {
        let help_labels = action_names(&Menu {
            name: "Help".into(),
            disabled: false,
            items: super::stcode_help_items(),
        });

        assert!(help_labels.contains(&"View Dependency Licenses".to_string()));
        assert!(help_labels.contains(&"Stcode Repository".to_string()));

        assert!(!help_labels.contains(&"View Release Notes Locally".to_string()));
        assert!(!help_labels.contains(&"View Telemetry".to_string()));
        assert!(!help_labels.contains(&"Show Welcome".to_string()));
        assert!(!help_labels.contains(&"File Bug Report...".to_string()));
        assert!(!help_labels.contains(&"Request Feature...".to_string()));
        assert!(!help_labels.contains(&"Zed Repository".to_string()));
        assert!(!help_labels.contains(&"Zed Twitter".to_string()));
        assert!(!help_labels.contains(&"Join the Team".to_string()));
    }
}
