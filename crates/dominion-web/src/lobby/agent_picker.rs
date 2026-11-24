//! Selects a house agent and its seat's runner strategy.

use super::who::effort_title;
use dioxus::prelude::*;
use dominion_protocol::lobby::{
    AgentSelection, HouseModel, InferenceSpeed, LaunchOptions, RunnerStrategy, house_choice,
};

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
/// several. The strategy follows the selection when the new profile supports it.
#[component]
pub fn AgentPicker(
    models: Vec<HouseModel>,
    value: Option<AgentSelection>,
    none: Option<String>,
    #[props(default)] class: String,
    #[props(default)] action: String,
    #[props(default)] title: String,
    #[props(default)] disabled: bool,
    onpick: EventHandler<Option<AgentSelection>>,
) -> Element {
    let options = value
        .as_ref()
        .map(AgentSelection::options)
        .unwrap_or_default();
    let strategy = options.strategy;
    let picked = value.as_ref().and_then(|v| house_choice(&models, v.name()));
    let clock_aware = picked.is_some_and(|(_, c)| c.supports_clock_aware);
    let speed_choice = picked.filter(|(_, c)| c.supports_speed).map(|(_, c)| {
        let fast = options
            .speed
            .map_or(c.fast_mode, |s| s == InferenceSpeed::Fast);
        (c.agent.clone(), fast)
    });
    let selected = value.as_ref().map(|v| v.name().to_owned());
    let current = picked.map(|(m, _)| (m.program.clone(), m.model.clone()));
    let mut programs: Vec<String> = vec![];
    for m in &models {
        if !programs.contains(&m.program) {
            programs.push(m.program.clone());
        }
    }
    let models_for_pick = models.clone();
    let efforts = picked.and_then(|(m, current)| {
        let choices: Vec<_> = m
            .choices
            .iter()
            .filter(|c| {
                !c.fast_mode
                    || !c.supports_speed
                    || !m
                        .choices
                        .iter()
                        .any(|other| !other.fast_mode && other.effort == c.effort)
            })
            .map(|c| {
                let mut label = c.effort.as_deref().map_or(c.agent.clone(), effort_title);
                if c.fast_mode && !c.supports_speed {
                    label.push_str(" · Fast");
                }
                (
                    c.agent.clone(),
                    label,
                    c.effort == current.effort
                        && (current.supports_speed || c.fast_mode == current.fast_mode),
                )
            })
            .collect();
        (choices.len() > 1).then_some(choices)
    });
    let models_for_effort = models.clone();
    let select = move |c: &dominion_protocol::lobby::HouseChoice| {
        AgentSelection::new(
            c.agent.clone(),
            LaunchOptions {
                strategy: if c.supports_clock_aware {
                    strategy
                } else {
                    RunnerStrategy::Classic
                },
                speed: options.speed.filter(|_| c.supports_speed),
            },
        )
    };
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
                    .map(select);
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
        if let Some(efforts) = efforts {
            select {
                class: "{class} lb-effort-select",
                "data-action": "pick-effort",
                title: "Reasoning effort",
                disabled,
                onchange: move |e| onpick.call(house_choice(&models_for_effort, &e.value()).map(|(_, c)| select(c))),
                for (name, label, is_selected) in efforts {
                    option { key: "{name}", value: "{name}", selected: is_selected, "{label}" }
                }
            }
        }
        if let Some((agent, fast)) = speed_choice {
            select {
                class: "{class} lb-effort-select",
                title: "Inference speed",
                aria_label: "Inference speed",
                "data-action": "pick-speed",
                disabled,
                onchange: move |e| {
                    let speed = if e.value() == "fast" { InferenceSpeed::Fast } else { InferenceSpeed::Standard };
                    onpick.call(Some(AgentSelection::new(agent.clone(), LaunchOptions { speed: Some(speed), ..options })));
                },
                option { value: "standard", selected: !fast, "Standard" }
                option { value: "fast", selected: fast, "Fast" }
            }
        }
        if clock_aware && let Some(agent) = selected {
            select {
                class: "{class} lb-effort-select",
                title: "Runner strategy",
                aria_label: "Runner strategy",
                "data-action": "pick-strategy",
                disabled,
                onchange: move |e| {
                    let strategy = if e.value() == "clock_aware" { RunnerStrategy::ClockAware } else { RunnerStrategy::Classic };
                    onpick.call(Some(AgentSelection::new(agent.clone(), LaunchOptions { strategy, ..options })));
                },
                option { value: "classic", selected: strategy.is_classic(), "Classic" }
                option { value: "clock_aware", selected: !strategy.is_classic(), "Clock-aware" }
            }
        }
    }
}
