# The providers whose models the server's agents offer, unless
# `services.dominion.agents.catalogs` says otherwise. Each is a `[[catalogs]]` entry of the
# runner's settings (README "catalogs"): every model it lists that `models` allows and
# `exclude` doesn't, at each effort it has, is an agent named after them ("Opus 5.5 High").
# The profiles in ./agents.nix keep their names for the models and efforts they name.
[
  {
    program = "claude";
    models = ["claude-opus-*" "claude-sonnet-*" "claude-fable-*"];
    exclude = ["claude-*-4-*"];
  }
  {
    program = "openai";
    models = ["gpt-6*"];
  }
  {
    program = "antigravity";
    models = ["gemini-4-*"];
  }
]
