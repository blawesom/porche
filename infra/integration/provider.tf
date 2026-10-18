# Credentials NEVER live in this repository. They come from the sandbox
# profile of ~/.osc/config.json (the only permitted account — see
# README.md / Makefile guard) and are exported by make:
#   export OSC_PROFILE=sandbox
#   export OSC_ACCESS_KEY=$(jq -r .sandbox.access_key ~/.osc/config.json)
#   export OSC_SECRET_KEY=$(jq -r .sandbox.secret_key ~/.osc/config.json)
#   export OSC_REGION=$(jq -r .sandbox.region ~/.osc/config.json)
provider "outscale" {
  region = var.region
}
