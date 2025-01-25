### Mattermost Team version cleanup utilities

Simplistic tool to clear self hosted mattermost from old files and entries in postgresql database

#### Requirements
- rust
- make (optional)

#### how to run
- run `make all`
- edit .env and modify to environment variables to suit your postgres installation and Mattermost directory
**or** 
`cargo build --release && cp target/release/mmjab ./mmjab && chmod +x ./mmjab` and run `./mmjab --help` to get explainer