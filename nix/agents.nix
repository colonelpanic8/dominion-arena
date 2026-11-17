# The server's agent profiles, unless `services.dominion.agents.profiles` says otherwise.
# Each is an `[[agents]]` entry of the runner's settings (README "agents"). Their
# guidance comes from the playbook they start in, not from `instructions`.
[
  {
    name = "Opus High";
    program = "claude";
    model = "claude-opus-5-5";
    effort = "high";
  }
  {
    name = "Fable High";
    program = "claude";
    model = "claude-fable-5-2";
    effort = "high";
  }
  {
    name = "Sonnet High";
    program = "claude";
    model = "claude-sonnet-5-5";
    effort = "high";
  }
  {
    name = "Sol High";
    program = "openai";
    model = "gpt-6.1-sol";
    effort = "high";
  }
  {
    name = "Astra High";
    program = "openai";
    model = "gpt-6-astra";
    effort = "high";
  }
  {
    name = "Terra High";
    program = "openai";
    model = "gpt-6.1-terra";
    effort = "high";
  }
  {
    name = "Argon High";
    program = "antigravity";
    model = "gemini-4-argon-high";
  }
]
