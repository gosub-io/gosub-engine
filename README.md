# Gosub Browser Engine

An embeddable, async browser engine written in Rust.

Join us on our development [Zulip chat](https://chat.developer.gosub.io), or our
[Discord server](https://chat.gosub.io) for general chat. If you'd like to contribute, start with
the [contribution guide](CONTRIBUTING.md).


## About

Gosub is a modular, embeddable browser engine. The primary entry point is `GosubEngine` in the
[`gosub_engine`](crates/gosub_engine) crate. You provide a render backend and a compositor; the
engine owns a multi-zone/tab model, an async networking stack, cookie and storage isolation per
zone, and an event bus. Your user-agent (UA) drives everything via `TabCommand` and reacts to
`EngineEvent`.

**Core components:**

| Crate | Role |
|---|---|
| `gosub_engine` | `GosubEngine` — the unified entry point |
| `gosub_interface` | Shared traits wiring the components together (the config system) |
| `gosub_html5` | HTML5 tokenizer / parser |
| `gosub_css3` | CSS3 tokenizer / parser |
| [`gosub-sonar`](https://github.com/gosub-io/gosub-sonar) | Networking stack (async, streaming, priority-scheduled) — external crate |
| `gosub_lattice` | CSS table layout |
| `gosub_render_pipeline` | Render pipeline — layout (Taffy), stages, tiling, compositor |
| `gosub_renderer_cairo` | Cairo render backend (CPU) |
| `gosub_renderer_skia` | Skia render backend (CPU / GPU) |
| `gosub_renderer_vello` | Vello / wgpu render backend (GPU) |
| `gosub_fontmanager` | Font system — text shaping and measurement |
| `gosub_jsapi` | Browser Web API implementations (console, fetch, DOM, …) |
| `gosub_v8` | V8 JavaScript engine bindings |
| `gosub_config` | Configuration store |

For the full crate listing see [`docs/crates.md`](docs/crates.md).


## Status

The engine is under active development. What works today:

- **Multi-zone / multi-tab model** — zones isolate cookies and storage; tabs are controlled via `TabCommand`
- **Async networking** — streaming HTTP fetcher with priority queues, inflight coalescing, redirect handling, and per-zone cookie isolation
- **Event-driven UA interface** — `EngineEvent` (navigation, resource, redraw) flows out; `TabCommand` / `EngineCommand` flow in
- **HTML5 and CSS3 parsing** — spec-compliant parsers for both
- **Pluggable render backends** — Null (headless), Cairo (GTK4), Skia, Vello (wgpu)


## Documentation

Start here, then dig into the topic you need.

**Getting started**

- [Tutorial](docs/tutorial.md) — start the engine, open a tab, navigate, handle events
- [Configuration](docs/configuration.md) — choosing a render backend and font system
- [Running the examples](docs/examples.md) — headless, GUI (winit / GTK4 / egui), and component tools
- [WebAssembly](docs/webassembly.md) — compile and run the engine in the browser
- [Development](docs/development.md) — tests and benchmarks

**Reference**

- [Crates](docs/crates.md) — the workspace crate layout
- [Component tools](docs/binaries.md) — the standalone `cargo run --bin …` tools

**Architecture**

- [Networking — architecture](docs/network/net-architecture.md) and [design notes](docs/network/net-design.md)
- [Cookies](docs/cookies.md)
- [Storage (local / session)](docs/datastores.md)
- [Pump](docs/network/pump.md) — moving HTTP stream data to targets
- [Render pipeline](docs/render-pipeline/README.md)


## Contributing

We welcome contributions. Because the engine is still taking shape, a lot of work is exploratory
— building proofs-of-concept, reading specs, and making architectural decisions — rather than
pure coding.

Join us on [Zulip](https://chat.developer.gosub.io) or [Discord](https://chat.gosub.io) before
diving in; it will save you time and help us keep things coordinated. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the details.


## 🌐 Web Resources & Interactive Index
- [ANACONDA RUNNER](https://ilearnworld.pages.dev/anaconda-runner.html)
- [CUTE ANIMAL WORLD](https://themindplaying.web.app/cute-animal-world.html)
- [YUMMY TALES 4](https://learnquesters.pages.dev/yummy-tales-4.html)
- [CATEGORY ARCHERY](https://learnquester.pages.dev/category-archery.html)
- [TIED UP](https://studyplayings.web.app/tied-up.html)
- [MERGE CUBES 2048 3D](https://learnquester.github.io/merge-cubes-2048-3d.html)
- [COP RUN 3D](https://quizverses.pages.dev/cop-run-3d.html)
- [CATEGORY PUZZLE 2](https://thelearnquester.web.app/category-puzzle-2.html)
- [MAZE CRAZE](https://learnquester.github.io/maze-craze.html)
- [WEDNESDAY ADDAMS BEAUTY SALON](https://thequizzone.pages.dev/wednesday-addams-beauty-salon.html)
- [FIND THE FROG HIDDEN OBJECTS](https://thelearnquesters.pages.dev/find-the-frog-hidden-objects.html)
- [TILE CONNECT CLUB](https://quizverses-9d2f2.web.app/tile-connect-club.html)
- [ELEVATOR FIGHT](https://themindzone.pages.dev/elevator-fight.html)
- [BUBBLE SHOOTER WILD WEST](https://studyplayings.pages.dev/bubble-shooter-wild-west.html)
- [BACK TO SCHOOL UNIFORMS EDITION](https://thequizzone.pages.dev/back-to-school-uniforms-edition.html)
- [GLOVES GROW RUSH](https://quizverses-9d2f2.web.app/gloves-grow-rush.html)
- [INDEX6](https://studyquesthub.web.app/index6.html)
- [SANDBOX ISLAND WAR](https://theskillquest.pages.dev/sandbox-island-war.html)
- [HOME PIN 1](https://thequizzone.pages.dev/home-pin-1.html)
- [FISH JAM](https://theskillquest.pages.dev/fish-jam.html)
- [CATEGORY CASUAL 4](https://thequizzone.pages.dev/category-casual-4.html)
- [MARBLE RUN ULTIMATE RACE](https://theskillquest.pages.dev/marble-run-ultimate-race.html)
- [DYNAMONS 11](https://learnquester.github.io/dynamons-11.html)
- [STICKMAN ZOMBIE VS STICKMAN HERO](https://learnquester.github.io/stickman-zombie-vs-stickman-hero.html)
- [BRAINSTORMING 2D](https://thelearnquesters.pages.dev/brainstorming-2d.html)
- [EMOJI SMASHER SMILEY GAME](https://learnquester.github.io/emoji-smasher-smiley-game.html)
- [CATEGORY STICKMAN](https://quizverses-9d2f2.web.app/category-stickman.html)
- [LIMOUSINE CAR GAME SIMULATOR](https://theskillquest.pages.dev/limousine-car-game-simulator.html)
- [DOP DRAW ONE PART](https://theskillquest.pages.dev/dop-draw-one-part.html)
- [K POP HUNTER HALLOWEEN FASHION](https://quizverses.github.io/k-pop-hunter-halloween-fashion.html)
- [ROBIN HOOD ARCHER](https://theskillquest.pages.dev/robin-hood-archer.html)
- [ASSASSIN COMMANDO CAR DRIVING](https://studyplayings.pages.dev/assassin-commando-car-driving.html)
- [CAPYBARA JUMP](https://learnquesters.pages.dev/capybara-jump.html)
- [CATEGORY CASUAL 12](https://iskillquest.pages.dev/category-casual-12.html)
- [CHICKEN BANANA RUN](https://studyquesthub.web.app/chicken-banana-run.html)
- [SORT MASTER](https://learnquester.github.io/sort-master.html)
- [DAILY MATCH](https://themindzone.pages.dev/daily-match.html)
- [MERGE TIKTOK GRAVITY KNIFE](https://thequizzone.pages.dev/merge-tiktok-gravity-knife.html)
- [PANDA RESTAURANT](https://thelearnquesters.pages.dev/panda-restaurant.html)
- [TRAFFIC ESCAPE PUZZLE](https://learnquester.pages.dev/traffic-escape-puzzle.html)
- [MERGE FRUIT](https://studyplaying.github.io/merge-fruit.html)
- [INDEX3](https://studyplayings.pages.dev/index3.html)
- [SLENDER BOY ESCAPE ROBBIE](https://thequizzone.pages.dev/slender-boy-escape-robbie.html)
- [CAT EVOLUTION 2](https://thelearnquesters.pages.dev/cat-evolution-2.html)
- [BUILDING MODS FOR MINECRAFT](https://learnquester.pages.dev/building-mods-for-minecraft.html)
- [CATEGORY ADVENTURE](https://themindzone.pages.dev/category-adventure.html)
- [JAILBREAK ROBLOX JUMPER](https://thelearnquesters.pages.dev/jailbreak-roblox-jumper.html)
- [ANTS PARTY](https://learnquester.github.io/ants-party.html)
- [TRICKY CASTLE](https://studyplaying.github.io/tricky-castle.html)
- [DRAW TO HOME 3D](https://thequizzone.pages.dev/draw-to-home-3d.html)
- [SNAKE PUZZLE ESCAPE](https://thelearnquesters.pages.dev/snake-puzzle-escape.html)
- [BUCKSHOT ROULETTE](https://theskillquest.pages.dev/buckshot-roulette.html)
- [CAKE SORT](https://themindzone.pages.dev/cake-sort.html)
- [CATEGORY BUILDING](https://themindplay.pages.dev/category-building.html)
- [UNPUZZLE MASTER](https://learnquester.github.io/unpuzzle-master.html)
- [MAGIC BRICK WARS](https://theskillquest.pages.dev/magic-brick-wars.html)
- [CUBE KING](https://studyquesthub.web.app/cube-king.html)
- [MAD TRUCK](https://studyplaying.github.io/mad-truck.html)
- [DRESS UP RUN](https://theskillquest.pages.dev/dress-up-run.html)
- [CATEGORY CASUAL971](https://studyplayings.pages.dev/category-casual971.html)
- [MATH KING MATH SKILL GAME](https://studyquests.pages.dev/math-king-math-skill-game.html)
- [ISLAND EXPANDER](https://studyplaying.github.io/island-expander.html)
- [CATEGORY WAR137](https://studyquesthub.web.app/category-war137.html)
- [SWEEPER CURLING](https://studyplaying.github.io/sweeper-curling.html)
- [CHICKEN WARS MERGE GUNS](https://studyplaying.github.io/chicken-wars-merge-guns.html)
- [STICKMAN DISMOUNT SIMULATOR](https://learnquester.github.io/stickman-dismount-simulator.html)
- [INDYGIRL AND THE GOLDEN SKULL](https://iskillquest.pages.dev/indygirl-and-the-golden-skull.html)
- [CATEGORY MISSION206](https://studyquesthub.web.app/category-mission206.html)
- [SLIDING GEMS](https://themindplay.github.io/sliding-gems.html)
- [HIDDEN OBJECTS ISLAND](https://studyplayings.web.app/hidden-objects-island.html)
- [CATEGORY RAGDOLL57](https://themindplay.pages.dev/category-ragdoll57.html)
- [STEAL ITEMS IO](https://theskillquest.pages.dev/steal-items-io.html)
- [DRAGON HUNTER](https://quizverses-9d2f2.web.app/dragon-hunter.html)
- [CLEAN THE FLOOR](https://iskillquest.pages.dev/clean-the-floor.html)
- [CATEGORY MERGE](https://studyquesthub.web.app/category-merge.html)
- [RACE CLICKER](https://theskillquest.pages.dev/race-clicker.html)
- [CATEGORY QUIZ40](https://themindplay.pages.dev/category-quiz40.html)
- [DRIVE RACE CRASH](https://learnquester.pages.dev/drive-race-crash.html)
- [MERGE BRAINROT](https://themindzone.pages.dev/merge-brainrot.html)
- [MATH BOX BALANCE](https://theskillquest.pages.dev/math-box-balance.html)
- [US ARMY CAR GAMES TRUCK DRIVING](https://theskillquest.pages.dev/us-army-car-games-truck-driving.html)
- [HOOP WORLD 3D](https://learnquester.pages.dev/hoop-world-3d.html)
- [POTION MERGE WITCH](https://themindplay.github.io/potion-merge-witch.html)
- [OBBY WITH FRIENDS DRAW AND JUMP](https://themindzone.pages.dev/obby-with-friends-draw-and-jump.html)
- [HEXA ARROWS PUZZLE](https://studyquests.pages.dev/hexa-arrows-puzzle.html)
- [CATEGORY ZOMBIE175](https://quizverses-9d2f2.web.app/category-zombie175.html)
- [AHA WORLD DREAM TOWN](https://themindplay.pages.dev/aha-world-dream-town.html)
- [BRAINROT MERGE](https://learnquester.pages.dev/brainrot-merge.html)
- [HORSEBACK SURVIVAL](https://quizverses.github.io/horseback-survival.html)
- [BFFS K POP FANGIRLS](https://learnquester.github.io/bffs-k-pop-fangirls.html)
- [ANIME COUPLE AVATAR MAKER](https://iskillquest.pages.dev/anime-couple-avatar-maker.html)
- [JELLY MATH 3D](https://learnquester.pages.dev/jelly-math-3d.html)
- [HAPPY TOWN](https://themindplay.github.io/happy-town.html)
- [DEVIL DUCK NOT A TROLL GAME](https://quizverses.github.io/devil-duck-not-a-troll-game.html)
- [CINEMA EMPIRE IDLE TYCOON](https://learnquester.github.io/cinema-empire-idle-tycoon.html)
- [CATEGORY POOL 2](https://studyplaying.github.io/category-pool-2.html)
- [MERGE CUBE CHALLENGE](https://themindplay.pages.dev/merge-cube-challenge.html)
- [OBBY 1 PET EVERY SECONDS](https://learnquester.github.io/obby-1-pet-every-seconds.html)
- [ROYAL COIN RUSH](https://studyquests.pages.dev/royal-coin-rush.html)
- [CATEGORY CASUAL 7](https://studyquests.github.io/category-casual-7.html)
- [ITALIAN BRAINROT QUIZ](https://studyquests.github.io/italian-brainrot-quiz.html)
- [CATEGORY STICKMAN 2](https://studyquests.github.io/category-stickman-2.html)
- [MAHJONG LINES](https://themindplay.github.io/mahjong-lines.html)
- [CATEGORY SOLITAIRE](https://themindzone.pages.dev/category-solitaire.html)
- [CIRCLE SHOOTER MASTER](https://quizverses.github.io/circle-shooter-master.html)
- [REAL STREET FIGHTER 3D](https://thelearnquesters.pages.dev/real-street-fighter-3d.html)
- [JOURNEY OF ESCAPE](https://theskillquest.pages.dev/journey-of-escape.html)
- [DAYCARE TYCOON](https://quizverses.github.io/daycare-tycoon.html)
- [YOUR DREAM ROOM](https://studyplayings.pages.dev/your-dream-room.html)
- [CATEGORY SPORTS](https://quizverses-9d2f2.web.app/category-sports.html)
- [SID GINNY Y2K GLAM CLASH](https://themindzone.pages.dev/sid-ginny-y2k-glam-clash.html)
- [MOBILE PHONE CASE DIY](https://theskillquest.pages.dev/mobile-phone-case-diy.html)
- [CATEGORY CASUAL 5](https://thelearnquesters.pages.dev/category-casual-5.html)
- [CATEGORY ROBOT49](https://themindplay.pages.dev/category-robot49.html)
- [TOWER WARS ARENA](https://themindzone.pages.dev/tower-wars-arena.html)
- [CATEGORY MISSION207](https://learnquester.github.io/category-mission207.html)
- [CATEGORY ARENA255](https://thelearnquesters.pages.dev/category-arena255.html)
- [CATEGORY RPG80](https://themindzone.pages.dev/category-rpg80.html)
- [CRAZY GOOSE SIMULATOR](https://thelearnquesters.pages.dev/crazy-goose-simulator.html)
- [GOODS TRIPLE MATCH 3D](https://studyquests.github.io/goods-triple-match-3d.html)
- [CATEGORY MOBILE2 095](https://studyplaying.github.io/category-mobile2-095.html)
- [WINTER MAZE](https://themindplay.github.io/winter-maze.html)
- [DUNK CHALLENGE](https://themindplay.github.io/dunk-challenge.html)
- [INDEX17](https://studyquests.github.io/index17.html)
- [OBBY TSUNAMI ESCAPE 1 BY CAR](https://studyplaying.github.io/obby-tsunami-escape-1-by-car.html)
- [POWERFUL PUNCH](https://theskillquest.pages.dev/powerful-punch.html)
- [BFFS Y2K FASHION](https://iskillquest.pages.dev/bffs-y2k-fashion.html)
- [FALLING ART RAGDOLL SIMULATOR](https://learnquester.github.io/falling-art-ragdoll-simulator.html)
- [ZUMBIA QUEST](https://thequizzone.pages.dev/zumbia-quest.html)
- [KNIT RESCUE](https://thelearnquesters.pages.dev/knit-rescue.html)
