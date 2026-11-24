//! Picking one of the server's agents: its model, then its effort.

use super::who::effort_title;
use dioxus::prelude::*;
use dominion_protocol::lobby::{HouseModel, house_choice};

/// `models` without the agents `taken`, and without the models left with none.
pub fn without(models: &[HouseModel], taken: &[String]) -> Vec<HouseModel> {
    let taken: Vec<String> = taken.iter().map(|t| t.to_lowercase()).collect();
    (models.iter())
        .filter_map(|m| {
            let mut m = m.clone();
            m.choices
                .retain(|c| !taken.contains(&c.agent.to_lowercase()));
            (!m.choices.is_empty()).then_some(m)
        })
        .collect()
}

/// A model select grouped by what runs them, and an effort select for a model that has
/// several. `onpick` gets the agent's name: a model's default effort when the model is
/// picked, and none for `none`, the first option, when there is one.
#[component]
pub fn AgentPicker(
    models: Vec<HouseModel>,
    value: Option<String>,
    none: Option<String>,
    #[props(default)] class: String,
    #[props(default)] action: String,
    #[props(default)] title: String,
    #[props(default)] disabled: bool,
    onpick: EventHandler<Option<String>>,
) -> Element {
    let picked = value.as_deref().and_then(|v| house_choice(&models, v));
    let current = picked.map(|(m, _)| (m.program.clone(), m.model.clone()));
    let mut programs: Vec<String> = vec![];
    for m in &models {
        if !programs.contains(&m.program) {
            programs.push(m.program.clone());
        }
    }
    let models_for_pick = models.clone();
    let efforts = picked.filter(|(m, _)| m.choices.len() > 1).map(|(m, c)| {
        let options: Vec<(String, String)> = (m.choices.iter())
            .map(|c| {
                let mut label = c.effort.as_deref().map_or(c.agent.clone(), effort_title);
                if c.fast_mode {
                    label.push_str(" · Fast");
                }
                (c.agent.clone(), label)
            })
            .collect();
        (options, c.agent.clone())
    });
    rsx! {
        select {
            class: "{class}",
            "data-action": "{action}",
            title: "{title}",
            disabled,
            onchange: move |e| {
                let v = e.value();
                let pick = v
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| models_for_pick.get(i))
                    .and_then(|m| m.default_choice())
                    .map(|c| c.agent.clone());
                onpick.call(pick);
            },
            if let Some(none) = none {
                option { value: "", selected: picked.is_none(), "{none}" }
            } else if picked.is_none() {
                option { value: "", selected: true, disabled: true, "Choose a model" }
            }
            for program in programs {
                optgroup { key: "{program}", label: "{program}",
                    for (i, m) in models.iter().enumerate().filter(|(_, m)| m.program == program) {
                        option {
                            key: "{i}",
                            value: "{i}",
                            selected: current.as_ref() == Some(&(m.program.clone(), m.model.clone())),
                            "{m.name}"
                        }
                    }
                }
            }
        }
        if let Some((options, agent)) = efforts {
            select {
                class: "{class} lb-effort-select",
                "data-action": "pick-effort",
                title: "Reasoning effort and speed",
                disabled,
                onchange: move |e| onpick.call(Some(e.value())),
                for (name, label) in options {
                    option { key: "{name}", value: "{name}", selected: name == agent, "{label}" }
                }
            }
        }
    }
}
