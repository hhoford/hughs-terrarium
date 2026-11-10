import { useEffect, useState } from "react";
import { Symbol } from "./Symbol";
export function keyHelp(code: string, queen: boolean, god=false): [string, string] {
  if(code==='KeyK')return ['Label radar position','K opens a label for your current position on the mini-globe. Right-click the radar to label another coordinate.'];
  if(god){
    const help:Record<string,[string,string]>={
      KeyW:['Ascend','W moves the God craft up.'],KeyS:['Descend','S moves down; the spectator is invincible.'],
      ArrowUp:['Forward','Up moves forward at 2 km/s.'],ArrowDown:['Backward','Down moves backward at 2 km/s.'],
      ArrowLeft:['Strafe left','Left moves sideways.'],ArrowRight:['Strafe right','Right moves sideways.'],
      ShiftLeft:['Global travel','Hold Shift for 100 km/s travel.'],ShiftRight:['Global travel','Hold Shift for 100 km/s travel.'],
      Space:['Observer','The God craft observes; it has no weapon.'],KeyH:['Station keeping','Release movement keys to stop the God craft.'],KeyT:['Observer','The God radar already sees all creatures, resources, tags and private trail/danger concentrations.'],KeyG:['Observer','Use Home to place resources. The God craft carries no cargo.'],KeyB:['Observer','The God craft belongs to no hive and carries no cargo.'],KeyM:['Observer','K adds a labeled radar tag. The God craft does not create weapon marks.']};
    if(help[code])return help[code];
  }
  const map: Record<string, [string, string]> = {
    KeyM: ["Mark location",queen ? "Only dragonflies mark coordinates. Fly close and slow, press M, then select that hive-only pin while piloting the queen. Marks last 60 seconds." : "M drops a fixed hive-only coordinate for 60 seconds. Below H+250 m or within 120 m of ground, at ≤20 m/s, gives a 10 m fix; otherwise error is 150 m. Only close fixes enable the 50 m precision core."],
    KeyT: [
      "Scan",
      queen
        ? "Only dragonflies scan. Their paid local scans and high-altitude resource surveys share resource discoveries with the hive."
        : "T scans 8 km locally, reaching 1,000 km at 80 km altitude. High surveys cost 10 MJ. Finds resource deposits and extends nearby creature detection. Discoveries are shared with your hive.",
    ],
    KeyG: [
      "Collect",
      "G or the down-tray collects a scanned deposit within 120 m. Slow below 20 m/s. Cargo holds 2,000 GJ and 2,000 kg. Carried energy is cargo; only a queen refuels your flight reserve.",
    ],
    KeyB: [
      "Deliver and refuel",
      "B or the up-tray transfers cargo to your queen within 300 m, then refills your flight energy from her reserve. Undelivered cargo stays aboard.",
    ],
    KeyW: [
      "Lift",
      queen
        ? "Hold W to climb. The massive queen accelerates slowly and spends energy."
        : "Hold W to climb. Thrust spends energy.",
    ],
    KeyS: ["Descend", "Hold S for downward thrust. Watch terrain clearance."],
    ArrowUp: [
      "Pitch forward",
      queen ? "Hold ↑ to tilt forward. Queen tilt and horizontal acceleration remain deliberately slow." : "Hold ↑ to tilt the nose down, up to 80°. Add W for thrust. A full mineral load accelerates more slowly.",
    ],
    ArrowDown: [
      "Pitch backward",
      "Hold ↓ to tilt the nose up and slow or move backward.",
    ],
    ArrowLeft: ["Bank left", "Hold ← to roll left. Thrust follows your tilt."],
    ArrowRight: [
      "Bank right",
      "Hold → to roll right. Thrust follows your tilt.",
    ],
    KeyA: ["Turn left", "Hold A to yaw left."],
    KeyD: ["Turn right", "Hold D to yaw right."],
    ShiftLeft: [
      "Boost",
      queen
        ? "Boost is for dragonflies. The queen keeps her slow force limit."
        : "Hold Shift with thrust for stronger acceleration and higher energy use.",
    ],
    ShiftRight: [
      "Boost",
      queen
        ? "The queen has no boost."
        : "Hold Shift with thrust for stronger acceleration.",
    ],
    KeyH: [
      "Altitude assistance",
      "H switches powered altitude assistance. A coloured altitude arrow means assistance is on. Turning it off allows gravity to pull you down.",
    ],
    KeyC: [
      "First person / chase",
      "C switches between the pilot’s view and an external chase camera.",
    ],
    KeyL: [
      "Camera direction lock",
      "L holds the camera’s world bearing while your creature turns. Press again to follow.",
    ],
    KeyQ: [
      "Orbit left",
      "Hold Q to orbit the chase camera left. It does not steer your creature.",
    ],
    KeyE: ["Orbit right", "Hold E to orbit the chase camera right."],
    KeyR: ["Raise camera", "Hold R to raise the chase viewing angle."],
    KeyF: ["Lower camera", "Hold F to lower the chase viewing angle."],
    KeyZ: ["Move camera closer", "Hold Z to zoom the chase camera in."],
    KeyX: ["Move camera farther", "Hold X to zoom the chase camera out."],
    KeyV: [
      "Reset camera",
      "V restores the chase camera’s default position and direction.",
    ],
    Space: queen
      ? [
          "Queen strike",
          "Select a dragonfly mark on the small globe, then press Space or the rocket. Charge above the radar envelope for 30 seconds, reserving 60% of full energy. Flight takes 1–5 minutes; aim stays fixed.",
        ]
      : [
          "Spit",
          "Hold Space to fire gravity-driven spit along your nose. One hit kills a dragonfly, including a friendly. Queens absorb spit unharmed.",
        ],
    Escape: ["Tutorial", "Only “I am tutored” leaves this lesson."],
    Tab: [
      "Keyboard focus",
      "Tab moves focus through the controls. Enter or Space explains a focused button.",
    ],
  };
  return (
    map[code] ?? [
      "Unbound key",
      "This key has no flight action in the testing sandbox.",
    ]
  );
}
export function controlHelp(label: string, queen: boolean): [string, string] {
  if(label==='Radar globe map')return ['Radar globe','Drag to rotate; scroll to zoom. Contacts follow your hive observations. Green concentrations are trails; orange concentrations are danger. K adds a label. God sees every hive.'];
  if(label.includes('God craft'))return ['God craft','An invincible spectator with an omniscient radar globe. W/S move vertically, arrows move horizontally, A/D turn, and Shift selects 100 km/s travel.'];
  if(label.includes('radar tag')||label==='Radar tag label')return ['Radar tags','Use the pin to label your position, or right-click the radar globe to label a coordinate. Click a label to edit or remove it. Creature tags are private to their hive.'];
  if(label==='Activate all hives'||label==='Deactivate all hives')return ['Hive switch','Green activates; red deactivates. This switch controls every living hive. Your piloted creature stays under manual control.'];
  if (label.includes("hive autonomy")) return ["Hive autonomy", "Green activates this hive; red deactivates it. The hive idol identifies whose switch this is. Piloting a creature gives you control; returning Home releases it. Off means powered holding."];

  if(label.startsWith("Starting ")||label.startsWith("Energy per mixed")||label.startsWith("Minerals per mixed")||label.includes("configured world")) return ["Starting world","Set hive/scout counts, queen stores, mixed-deposit counts and amounts, and a repeatable seed. Energy fields use terajoules; queen minerals use tonnes. The reset symbol creates the configured world."];
  if(label.startsWith("Resource drop count")) return ["Scatter food","One places a deposit at your click. A larger count scatters that many deposits across land without replacing the world. Quantities are per deposit."];
  if(label==="Found queen") return ["Found queen","Orphans remain dragonflies. Land, gather 1000 kg and 204.5 GJ total energy, then build for 120 seconds. No supplies are granted."];
  if(label==="Grow queen") return ["Grow queen","While grounded, grow toward twice your mass. Each added kg costs 1 kg minerals and 5 MJ. Collect local supplies to continue. Larger queens store more and reproduce faster."];
  if(label==="Lift founder") return ["Lift founder","Leave the ground using powered flight. Founding must finish before you lift."];
  if(label==="Land founder") return ["Land founder","Settle your orphan dragonfly safely onto land before founding."];
  if(label==="Refuel founder from cargo") return ["Refuel founder","Orphans can transfer carried energy into their flight tank. Ordinary workers refuel at their queen."];
  if(label.includes("starting world")) return ["Starting world","Open the starting configuration, or close it without changing the current world. Only the generate symbol creates a new world."];
  if (label.startsWith("Altitude assistance")) return keyHelp("KeyH", queen);
  if (label.startsWith("Global radar"))
    return [
      "Global radar",
      "The radar covers all Earth. Foreign members appear above the Gaussian-smoothed terrain surface +1,000 m. Drag the little globe to inspect Earth. Pins are hive-only dragonfly marks; select a pin to aim a queen strike.",
    ];
  if (label === "Map")
    return [
      "Point and click",
      queen
        ? "Select a dragonfly pin on the small globe to fix a strike coordinate, then use the rocket or Space. Terrain and creature clicks do not aim weapons."
        : "Use keys to fly, M to mark your location, and T to scan. Place testing members and resources from the Home globe.",
    ];
  if (label === "Flight attitude")
    return [
      "Attitude reticle",
      "The central lines show your bank and pitch. They follow the creature, independently of camera orbit.",
    ];
  if (label.startsWith("Hive ") && label.includes("perspective"))
    return [
      "Your hive",
      "This colour identifies the hive whose member you are piloting.",
    ];
  if (label.startsWith("Target "))
    return [
      "Radar contact",
      queen
        ? "Only dragonfly marks can be targeted. Contacts show positions; clicking a mark selects its fixed coordinate. The missile never chases units."
        : "Foreign members appear globally above the smoothed terrain surface +1,000 m. Friendly telemetry is always known.",
    ];
  if (label.startsWith("Resource ") || label.startsWith("Scanned resource"))
    return [
      "Resource deposit",
      "Globe symbols show every deposit. In flight only scanned deposits appear; the nearby number is distance in metres. Approach, collect with G, then deliver to the queen with B.",
    ];
  if (label.includes("drop in"))
    return [
      "Drop quantity",
      "Set the numeric amount before choosing the resource symbol. Energy is in billions of joules; minerals in kilograms. Then click a globe location.",
    ];
  if (label.startsWith("Cargo"))
    return [
      "Cargo",
      "The tray shows carried energy in billions of joules and minerals in kilograms. Queens show their stored mineral stock. Deliver with B within 300 m.",
    ];
  if (label.startsWith("Select dragonfly mark")) return ["Dragonfly mark","A 60-second coordinate shared within your hive. Tight marks enable precision against hollows. Select one, then fire; any exposed creature touched by the blast dies, at any altitude."];
  if (label.startsWith("Radar ")) return ["Radar globe","Drag to rotate the mapped globe. Coloured insects are contacts; the pin is a targetable dragonfly mark. Click your insect below to recenter."];
  if (label.startsWith("Nuclear protection") || label.startsWith("Nuclear shelter"))
    return [
      "Depression shelter",
      "Amber shield: below H−500 m, general-blast cover. A close fix enables a 50 m precision core that defeats hollow cover. Otherwise blast contact kills at any altitude. Radar starts at H+1,000 m.",
    ];
  if (label.startsWith("Energy"))
    return [
      "Energy",
      "The bar is your remaining reserve. Flying, powered holding and weapons spend energy.",
    ];
  if (
    label.startsWith("Terrain clearance") ||
    label.includes("radar altitude") ||
    label.includes("Visible on global")
  )
    return [
      "Terrain clearance",
      "The upward-arrow number is height above actual ground. The radar dot uses the invisible Gaussian terrain surface +1,000 m, rather than ground directly underneath.",
    ];
  if (label.startsWith("Speed"))
    return ["Speed", "Your speed is measured in metres per second."];
  const items: Record<string, [string, string]> = {
    "Mark dragonfly location":keyHelp("KeyM",false),
    "Centre radar on pilot":["Your position","Recentre the tiny mapped globe on your current position. Drag it to inspect the far side."],
    "Reproduce dragonfly":["Reproduce","While grounded, spend 100 GJ and 100 kg minerals. The newborn includes 75 GJ fuel. Small queens brood more slowly than mature queens. Taking off abandons the brood and its spent resources."],
    "Scan for resources": keyHelp("KeyT", false),
    "Collect nearby resource": keyHelp("KeyG", false),
    "Deliver cargo and refuel": keyHelp("KeyB", false),
    "Drop energy resource": [
      "Place energy",
      "Place a finite fuel deposit on terrain. The adjacent number sets billions of joules. Only the master globe knows unscanned locations.",
    ],
    "Drop mineral resource": [
      "Place minerals",
      "Place minerals on terrain; the adjacent number sets kilograms. Dragonflies must scan, collect and deliver them. Resources do not spawn randomly.",
    ],
    "Global home": [
      "Globe",
      "Return to the globe. The creature physically brakes and holds where you left it.",
    ],
    "Switch first person and chase camera": keyHelp("KeyC", queen),
    "Lock camera world direction": keyHelp("KeyL", queen),
    "Add a member": [
      "Add a testing member",
      "The plus places creatures, energy or minerals. Choose a symbol and click the globe. The editor is on the Home globe.",
    ],
    "Add queen": [
      "Add queen",
      "Create a new hive with one queen. Choose her position on the Home globe.",
    ],
    "Add dragonfly to selected hive": [
      "Add dragonfly",
      "Create a dragonfly belonging to your selected hive. It holds position with a large finite reserve.",
    ],
    "Settle queen": [
      "Land the queen",
      "The down arrow descends at about 2 m/s. Grounded takeoff needs 389 seconds of preparation. Deep hollows block general blasts; a close dragonfly fix enables a precision strike. Altitude never grants blast immunity.",
    ],
    "Lift queen": [
      "Lift the queen",
      "The up arrow prepares for 389 seconds, then lifts slowly. Steering returns once clear of the ground, around 20 m.",
    ],
    "Charge queen strike at selected dragonfly mark": keyHelp("Space", true),
    "Cancel queen strike charge": [
      "Cancel charge",
      "Cancel before launch to recover 75% of the reserved energy. A launched missile cannot be recalled.",
    ],
    "Fire dragonfly spit": keyHelp("Space", false),
    "Map source attribution": [
      "Source credits",
      "The information symbol opens terrain and map attribution during normal play.",
    ],
  };
  return (
    items[label] ?? [
      "Control",
      label || "Click a control or press a key to learn what it does.",
    ]
  );
}
export function Tutorial({
  queen,
  god=false,
  finish,
}: {
  queen: boolean;
  god?:boolean;
  finish: () => void;
}) {
  const [message, setMessage] = useState<[string, string]>([
    "Try the controls",
    "Press a key or click any control. Nothing moves or fires until you choose “I am tutored”.",
  ]);
  useEffect(() => {
    let lastPointer = -Infinity;
    const explain = (event: Event) => {
      const target = event.target instanceof Element ? event.target : null;
      if (
        target?.closest("[data-tutorial-exit]") &&
        (!(event instanceof KeyboardEvent) ||
          ["Enter", "Space", "Tab"].includes(event.code))
      )
        return;
      if (event.type === "click" && performance.now() - lastPointer < 800) {
        event.preventDefault();
        event.stopImmediatePropagation();
        return;
      }
      if (event.type === "pointerdown") lastPointer = performance.now();
      if (event instanceof KeyboardEvent) {
        if (event.type === "keyup") {
          event.preventDefault();
          event.stopImmediatePropagation();
          return;
        }
        if (event.repeat) {
          event.preventDefault();
          event.stopImmediatePropagation();
          return;
        }
        const button = target?.closest("button,summary");
        setMessage(
          (event.code === "Enter" || event.code === "Space") && button
            ? controlHelp(button.getAttribute("aria-label") ?? "", queen)
            : keyHelp(
                event.shiftKey && ["KeyW", "KeyS"].includes(event.code)
                  ? "ShiftLeft"
                  : event.code,
                queen,god,
              ),
        );
        if (event.code === "Tab") return;
      } else if (event.type === "wheel")
        setMessage([
          "Camera distance",
          "In flight, hold Z / X to move the camera closer / farther. The mouse wheel zooms the Home globe or mini-globe radar.",
        ]);
      else {
        const control = target?.closest(
          "button,summary,[aria-label],[data-tutorial]",
        );
        if (control)
          setMessage(
            controlHelp(
              control.getAttribute("aria-label") ??
                control.getAttribute("data-tutorial") ??
                "",
              queen,
            ),
          );
        else
          setMessage([
            "Point and click",
            queen
              ? "Select a dragonfly pin on the tiny globe, then use the rocket or Space. The marked coordinate stays fixed."
              : "Use the keyboard to fly. Drag the little globe to inspect contacts; press M to mark your location.",
          ]);
      }
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    const events = [
      "keydown",
      "keyup",
      "pointerdown",
      "click",
      "dblclick",
      "contextmenu",
      "wheel",
      "touchstart",
    ];
    events.forEach((name) =>
      window.addEventListener(name, explain, { capture: true, passive: false }),
    );
    return () =>
      events.forEach((name) => window.removeEventListener(name, explain, true));
  }, [queen,god]);
  return (
    <>
      <aside
        className="tutorial-reference"
        aria-label="Tutorial control reference"
      >
        <header>
          <Symbol name={queen ? "queen" : "dragonfly"} size={27} />
          <strong>{queen ? "Queen" : "Dragonfly"}</strong>
          <span>Flight paused</span>
        </header>
        <dl>
          <dt>W / S</dt>
          <dd>Climb / descend</dd>
          <dt>↑ ↓ ← →</dt>
          <dd>Pitch / bank</dd>
          <dt>A / D</dt>
          <dd>Turn left / right</dd>
          <dt>Shift</dt>
          <dd>{queen ? "Dragonflies only" : "Stronger thrust"}</dd>
          <dt>H</dt>
          <dd>Powered altitude assistance</dd>
          <dt>Space</dt>
          <dd>{queen ? "Charge aimed strike" : "Fire spit"}</dd>
          <dt>C / L</dt>
          <dd>Camera view / direction lock</dd>
          <dt>Q / E</dt>
          <dd>Orbit camera</dd>
          <dt>R / F</dt>
          <dd>Raise / lower camera</dd>
          <dt>Z / X</dt>
          <dd>Camera closer / farther</dd>
          <dt>V</dt>
          <dd>Reset camera</dd>
          {!queen && (
            <>
              <dt>T / M / G / B</dt>
              <dd>Scan / mark / collect / deliver</dd>
            </>
          )}
          <dt>Click</dt>
          <dd>
            {queen
              ? "Select a dragonfly mark"
              : "Rotate the radar globe"}
          </dd>
        </dl>
        <p>
          Click any symbol for its explanation. Controls are available even when
          an action would normally be unavailable.
        </p>
      </aside>
      <section
        className="tutorial-explanation"
        aria-live="polite"
        aria-label="Control explanation"
      >
        <h2>{message[0]}</h2>
        <p>{message[1]}</p>
        <button data-tutorial-exit onClick={finish}>
          I am tutored
        </button>
      </section>
    </>
  );
}
