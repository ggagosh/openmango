#!/usr/bin/env bash
# Refresh the bundled model catalogue (assets/ai-models.json) from models.dev.
#
# The app ships this snapshot so model names, context sizes and prices are right
# offline; the model picker refreshes the same data at runtime. Keeping the file
# in models.dev's own shape lets one parser read both.
#
# Usage: scripts/update_ai_models.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/assets/ai-models.json"

# models.dev keys Gemini under "google"; Ollama serves local models and is listed
# through its own /api/tags instead.
providers='["anthropic","openai","google"]'

curl -fsSL https://models.dev/api.json |
	jq -S --argjson providers "$providers" '
    with_entries(select(.key as $key | $providers | index($key)))
    | map_values({
        id,
        name,
        models: (
          .models
          | map_values(select(
              .tool_call == true
              and (.status // "") != "deprecated"
              # Chat models only: no image or audio generators, no live/realtime variants,
              # and no open-weight models that these APIs do not actually serve.
              and (.modalities.output == ["text"])
              and (.modalities.input | index("text"))
              and (.open_weights // false) == false
            ))
          | map_values({
              id,
              name,
              tool_call,
              reasoning,
              structured_output,
              release_date,
              status,
              limit,
              modalities,
              open_weights,
              cost: ((.cost // {}) | {input, output}),
            })
        ),
      })' >"$out"

printf 'wrote %s (%s bytes, %s models)\n' \
	"$out" "$(wc -c <"$out" | tr -d ' ')" \
	"$(jq '[.[].models | length] | add' "$out")"
