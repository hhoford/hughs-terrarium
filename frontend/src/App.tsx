import {
  useCallback,
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import maplibregl from "maplibre-gl";
import { buildStyle } from "./style";
import { Symbol, type Glyph } from "./Symbol";
import {HiveEmblem,hiveEmblemMarkup} from './HiveEmblem';
import { GlobalRadar } from "./GlobalRadar";
import {GOD_ID,godBody} from "./flight/god";
import { LabFlight } from "./LabFlight";
import { LabWorld } from "./LabWorld";
import { StartPanel } from "./StartPanel";
import {
  colour,
  labState,
  resetLab,
  feedLab,
  labCommand,
  labTerrain,

  LabError,
  offset,
  distance,
  type Lab,
  type Pos,
} from "./lab";
import { Tutorial } from "./Tutorial";
import { unlockSound, signature } from "./sound";
const LessonContext = createContext(false);
const glyphMarkup = (queen: boolean) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${queen ? '<path d="M5 10 3 3l6 4 3-6 3 6 6-4-2 7M6 11l-3 6 5-1 4 7 4-7 5 1-3-6"/><ellipse cx="12" cy="13" rx="5" ry="6"/>' : '<path d="M12 7v15M11 8C-1 0-1 12 11 11M13 8C25 0 25 12 13 11M11 12C1 9 1 19 11 15M13 12C23 9 23 19 13 15"/><circle cx="12" cy="4" r="2"/>'}</svg>`;
function Icon({
  icon,
  label,
  onClick,
  active = false,
  disabled = false,
  children,
}: {
  icon: Glyph;
  label: string;
  onClick: () => void;
  active?: boolean;
  disabled?: boolean;
  children?: React.ReactNode;
}) {
  const teaching = useContext(LessonContext);
  return (
    <button
      className={`icon-button ${active ? "active" : ""}`}
      aria-label={label}
      onClick={onClick}
      disabled={disabled && !teaching}
    >
      <Symbol name={icon} />
      {children}
    </button>
  );
}
function AutonomySwitch({active,disabled=false,label,onClick,hive,lineage}:{active:boolean;disabled?:boolean;label:string;onClick:()=>void;hive?:number;lineage?:number[]}){
  const teaching=useContext(LessonContext);
  return <button className={`autonomy-switch ${active?'enabled':''}`} role="switch" aria-checked={active}
    aria-label={label} disabled={disabled&&!teaching} onClick={onClick}>
    {hive===undefined?<Symbol name="swarm" size={21}/>:<HiveEmblem hive={hive} lineage={lineage} size={28}/>}
    <span className="switch-track" aria-hidden="true"><i/></span>
  </button>;
}
function Meter({
  ratio,
  icon,
  label,
}: {
  ratio: number;
  icon: Glyph;
  label: string;
}) {
  return (
    <div
      className="meter"
      role="meter"
      aria-label={label}
      aria-valuenow={Math.round(ratio * 100)}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <Symbol name={icon} size={18} />
      <span>
        <i style={{ width: `${Math.max(0, Math.min(1, ratio)) * 100}%` }} />
      </span>
    </div>
  );
}
export default function App() {
  const host = useRef<HTMLDivElement>(null),
    map = useRef<maplibregl.Map | null>(null),
    flight = useRef<LabFlight | null>(null),
    world = useRef<LabWorld | null>(null),
    data = useRef<Lab | null>(null),
    selectedRef = useRef<number | null>(null),
    pilotRef = useRef<string | null>(null),
    targetRef = useRef<Pos | null>(null),
    targetMarkRef = useRef<number|null>(null),
    placeRef = useRef<"queen" | "dragonfly" | "energy" | "mineral" | null>(
      null,
    );
  const feedCount=useRef(1);
  const dropAmount = useRef({ energy: 250, mineral: 1000 });
  const armedRef = useRef(localStorage.getItem("hive-tutorial") === "1"),
    lessonRef = useRef(false);
  const [armed, setArmed] = useState(armedRef.current),
    [lesson, setLesson] = useState(false);
  const transition = useRef(0),
    release = useRef<Promise<unknown>>(Promise.resolve());
  const boot = useRef<Promise<Lab> | null>(null);
  const homeRef = useRef<() => void>(() => {});
  const [startOpen, setStartOpen] = useState(false);
  const [hollows, setHollows] = useState<"terrain" | "hollows">("terrain"),
    hollowsRef = useRef<"terrain" | "hollows">("terrain");
  const hollowsIdle = useRef<(() => void) | null>(null),
    hollowsFetching = useRef(false),
    hollowsDirty = useRef(false);
  const markers = useRef(new Map<string, maplibregl.Marker>()),
    shots = useRef(new Set<string>());
  const [game, setGame] = useState<Lab | null>(null),
    [god,setGod]=useState<ReturnType<typeof godBody>|null>(null),
    [hive, setHive] = useState<number | null>(null),
    [pilotId, setPilotId] = useState<string | null>(null),
    [target, setTarget] = useState<Pos | null>(null),
    [camera, setCamera] = useState("follow"),
    [menu, setMenu] = useState(false),
    [place, setPlace] = useState<
      "queen" | "dragonfly" | "energy" | "mineral" | null
    >(null),
    [busy, setBusy] = useState(true),
    [error, setError] = useState(""),
    [ready, setReady] = useState(false);
  const accept = useCallback((g: Lab) => {
    if (data.current && g.epoch < data.current.epoch) return;
    const replaced = data.current && g.epoch !== data.current.epoch;
    if (
      data.current &&
      g.epoch === data.current.epoch &&
      g.time < data.current.time
    )
      return;
    if (data.current?.epoch === g.epoch) {
      const previous = new Map(data.current.units.map(u => [u.id, u]));
      g = {
        ...g,
        units: g.units.map((u) => {
          const old = previous.get(u.id);
          return old && old.sample_time > u.sample_time ? old : u;
        }),
      };
    }
    const controlled=g.units.find(u=>u.id===pilotRef.current);
    if(controlled && controlled.hive!==selectedRef.current){selectedRef.current=controlled.hive;setHive(controlled.hive);}
    data.current = g;
    setGame(g);
    if (replaced) {
      lessonRef.current = false;
      setLesson(false);
      shots.current.clear();

      homeRef.current();
      return;
    }
    flight.current?.update(g);
    for (const b of g.bursts) {
      const id = `${g.epoch}-b${b.id}`;
      if (!shots.current.has(id)) {
        shots.current.add(id);
        if (b.kind === "nuclear") signature("impact");
      }
    }
    for (const s of g.missiles) {
      const id = `${g.epoch}-s${s.id}`;
      if (!shots.current.has(id)) {
        shots.current.add(id);
        signature("launch");
      }
    }
  }, []);
  const selectHive = useCallback((h: number) => {
    const g = data.current,
      q = g?.units.find((u) => u.hive === h && (u.kind === "queen" || u.founder));
    selectedRef.current = h;
    setHive(h);
    if (q && map.current)
      map.current.easeTo({
        center: [q.pos.lon, q.pos.lat],
        zoom: 2.1,
        bearing: 0,
        pitch: 0,
        duration: 900,
      });
  }, []);
  const home = useCallback(() => {
    if (lessonRef.current) return;
    transition.current++;
    setBusy(false);
    if (flight.current) release.current = flight.current.exit();
    flight.current = null;
    world.current?.destroy();
    world.current = null;
    pilotRef.current = null;
    setPilotId(null);
    targetRef.current = null;
    targetMarkRef.current=null;
    setTarget(null);
    setCamera("follow");
    setMenu(false);
    placeRef.current = null;
    setPlace(null);
    const m = map.current;
    if (!m) return;
    m.stop();
    m.setTerrain(null);
    if (hollowsIdle.current) {
      m.off("moveend", hollowsIdle.current);
      hollowsIdle.current = null;
    }
    if (m.getLayer("hollows-layer")) m.removeLayer("hollows-layer");
    if (m.getSource("hollows")) m.removeSource("hollows");
    hollowsFetching.current = false;
    hollowsDirty.current = false;
    hollowsRef.current = "terrain";
    setHollows("terrain");
    m.setCenterClampedToGround(true);
    m.setProjection({ type: "globe" });
    m.setMaxZoom(3.4);
    m.setMinZoom(0.5);
    const q = data.current?.units.find(
      (u) => u.hive === selectedRef.current && (u.kind === "queen" || u.founder),
    );
    m.jumpTo({
      center: [q?.pos.lon ?? m.getCenter().lng, 0],
      zoom: 2,
      pitch: 0,
      bearing: 0,
      roll: 0,
      elevation: 0,
    });
    void labState()
      .then((g) => {
        if (!pilotRef.current) accept(g);
      })
      .catch((e) => setError(String(e)));
  }, [accept]);
  homeRef.current = home;
  const enterHollows = useCallback(() => {
    const m=map.current;if(!m)return;
    m.setMaxZoom(14);
    if(!m.getSource("hollows"))m.addSource("hollows",{type:"raster",tiles:["/api/lab/hollows/{z}/{x}/{y}?geo=geography-v1-equal250-gauss5000-cut20000-hollows1"],tileSize:256,maxzoom:8,attribution:"Gaussian terrain hollow overview; detail refines with zoom"});
    if(!m.getLayer("hollows-layer"))m.addLayer({id:"hollows-layer",type:"raster",source:"hollows",paint:{"raster-opacity":0.85,"raster-fade-duration":0,"raster-resampling":"nearest"}});
  }, []);
  const toggleHollows = useCallback(() => {
    const next = hollowsRef.current === "hollows" ? "terrain" : "hollows";
    hollowsRef.current = next;
    setHollows(next);
    if (next === "hollows") enterHollows();
    else home();
  }, [enterHollows, home]);
  const send = useCallback(
    async (
      action: string,
      unit?: string,
      p?: Pos,
      h?: number,
      amount?: number,
      mark?:number,
    ) => {
      const g = data.current;
      if (!g || (lessonRef.current && action !== "tutorial_end")) return;
      setError("");
      try {
        const result = await labCommand(
          g.epoch,
          action,
          unit,
          p,
          h ?? selectedRef.current ?? undefined,
          amount,
          mark,
        );
        accept(result);
        return result;
      } catch (e) {
        setError(String(e));
      }
    },
    [accept],
  );
  const strike = useCallback(() => {
    if (lessonRef.current) return;
    const g = data.current,
      u = g?.units.find((u) => u.id === pilotRef.current);
    if (!u || u.kind !== "queen") return;
    if (u.charge) {
      void send("cancel", u.id, undefined, u.hive);
    } else if (targetMarkRef.current!==null) {
      void send("strike",u.id,undefined,u.hive,undefined,targetMarkRef.current);
    }
  }, [send]);
  const add = useCallback(
    async (kind: "queen" | "dragonfly" | "energy" | "mineral", p: Pos) => {
      setBusy(true);
      const h = selectedRef.current;
      if (kind === "dragonfly" && h === null) {
        setBusy(false);
        return;
      }
      const g = await send(
        kind === "energy"
          ? "drop_energy"
          : kind === "mineral"
            ? "drop_minerals"
            : kind === "queen"
              ? "add_queen"
              : "add_dragonfly",
        undefined,
        p,
        h ?? undefined,
        kind === "energy"
          ? dropAmount.current.energy * 1e9
          : kind === "mineral"
            ? dropAmount.current.mineral
            : undefined,
      );
      setBusy(false);
      placeRef.current = null;
      setPlace(null);
      setMenu(false);
      if (g && kind === "queen" && !pilotRef.current) {
        const q = g.units.filter((u) => u.kind === "queen").at(-1);
        if (q) selectHive(q.hive);
      }
    },
    [selectHive, send],
  );
  useEffect(() => {
    const style = buildStyle();
    style.terrain = undefined;
    style.projection = { type: "globe" };
    style.sources.terrarium = {
      type: "raster-dem",
      tiles: ["/api/lab/dem/{z}/{x}/{y}?geo=geography-v1-equal250-gauss5000-cut20000-hollows1"],
      encoding: "terrarium",
      tileSize: 256,
      maxzoom: 15,
    };
    const topo = style.layers.find((l) => l.id === "topo");
    if (topo && topo.type === "raster")
      topo.paint = { ...topo.paint, "raster-saturation": -0.65 };
    const m = new maplibregl.Map({
      container: host.current!,
      style,
      center: [0, 0],
      zoom: 2,
      minZoom: 0.5,
      maxZoom: 3.4,
      pitch: 0,
      bearing: 0,
      pixelRatio: 1,
      attributionControl: false,
    });
    map.current = m;
    let disposed = false,
      polling = false;
    let pollError = "", bootRetryAt = 0;
    m.on("load", () => {
      if (!disposed) setReady(true);
    });
    m.on("click", (e) => {
      if (lessonRef.current) return;
      if (pilotRef.current) return;
      if (placeRef.current)
        void add(placeRef.current, {
          lat: e.lngLat.lat,
          lon: e.lngLat.lng,
          alt: 0,
        });
    });
    const initialise = async () => {
      try {
        const g = await (boot.current ??= labState());
        if (disposed) return;
        const recovered = pollError; pollError = "";
        if (recovered) setError(current => current === recovered ? "" : current);
        accept(g);
        const q = g.units.find((u) => u.kind === "queen");
        if (q) m.jumpTo({ center: [q.pos.lon, 0], zoom: 2 });
      } catch (e) {
        boot.current = null;
        bootRetryAt = Date.now() + 1000;
        pollError = String(e);
        if (!disposed) setError(pollError);
      } finally {
        if (!disposed) setBusy(false);
      }
    };
    void initialise();
    const timer = setInterval(() => {
      if (polling || disposed) return;
      if (!data.current) {
        if (Date.now() < bootRetryAt) return;
        polling = true;
        void initialise().finally(() => { polling = false; });
        return;
      }
      polling = true;
      const perspective = pilotRef.current && pilotRef.current!==GOD_ID
        ? (selectedRef.current ?? undefined)
        : undefined;
      void labState(perspective,pilotRef.current===GOD_ID?undefined:pilotRef.current??undefined)
        .then((g) => {
          if (!disposed && pollError) {
            const recovered = pollError; pollError = "";
            setError(current => current === recovered ? "" : current);
          }
          if (
            !disposed &&
            perspective ===
              (pilotRef.current && pilotRef.current!==GOD_ID
                ? (selectedRef.current ?? undefined)
                : undefined)
          )
            accept(g);
        })
        .catch((e) => {
          pollError = String(e);
          if (!disposed) setError(pollError);
        })
        .finally(() => (polling = false));
    }, 120);
    return () => {
      disposed = true;
      clearInterval(timer);
      flight.current?.exit();
      world.current?.destroy();
      markers.current.forEach((x) => x.remove());
      markers.current.clear();
      m.remove();
      map.current = null;
    };
  }, [accept, add]);
  useEffect(() => {
    const m = map.current;
    if (!m || !ready || !game) return;
    const seen = new Set<string>();
    const droneGroups = new Map<number, string[]>();
    for (const unit of game.units) if (unit.kind === "dragonfly") {
      let group = droneGroups.get(unit.hive);
      if (!group) { group = []; droneGroups.set(unit.hive, group); }
      group.push(unit.id);
    }
    const piloted = pilotId===GOD_ID?flight.current?.body:game.units.find(u => u.id === pilotId);
    if (pilotId && !piloted) return;
    if (!pilotId) {
      for (const u of game.units) {
        if (u.kind !== "queen" && !u.founder && u.hive !== hive) continue;
        seen.add(u.id);
        let marker = markers.current.get(u.id);
        if (!marker) {
          const el = document.createElement("button");
          el.className = `globe-member ${u.kind}`;
          el.innerHTML = u.kind === 'queen'||u.founder ? hiveEmblemMarkup(u.hive,u.lineage) : glyphMarkup(false);
          el.setAttribute(
            "aria-label",
            u.kind === "queen"
              ? `Select hive ${u.hive + 1}`
              : `Select dragonfly ${u.id}`,
          );
          el.onclick = (e) => {
            e.stopPropagation();
            if (placeRef.current) return;
            if (!data.current?.units.find((x) => x.id === u.id)?.alive) {
              selectHive(u.hive);
              return;
            }
            const live=data.current?.units.find(x=>x.id===u.id);
            if (live && (live.kind === "queen" || live.founder) && selectedRef.current !== live.hive)
              selectHive(live.hive);
            else void enterRef.current(u.id);
          };
          marker = new maplibregl.Marker({
            element: el,
            opacityWhenCovered: 0,
            subpixelPositioning: true,
          })
            .setLngLat([u.pos.lon, u.pos.lat])
            .addTo(m);
          markers.current.set(u.id, marker);
        }
        if (marker.getLngLat().lng !== u.pos.lon || marker.getLngLat().lat !== u.pos.lat)
          marker.setLngLat([u.pos.lon, u.pos.lat]);
        if (u.kind === "dragonfly") {
          const group = droneGroups.get(u.hive)!;
          const index = group.indexOf(u.id), count = group.length;
          const angle =
            -Math.PI / 2 + (index * 2 * Math.PI) / Math.max(3, count);
          marker.setOffset([Math.cos(angle) * 58, Math.sin(angle) * 58]);
          marker
            .getElement()
            .style.setProperty("--leader-angle", `${angle + Math.PI}rad`);
        }

        if(u.kind==='queen'||u.founder){
          const identity=`${u.hive}:${u.lineage?.join(',')??''}`;
          if(marker.getElement().dataset.identity!==identity){marker.getElement().innerHTML=hiveEmblemMarkup(u.hive,u.lineage);marker.getElement().dataset.identity=identity;}
        }
        marker.getElement().style.color = u.alive ? colour(u.hive) : "#f38182";
        marker.getElement().classList.toggle("dead", !u.alive);
        if (!u.alive && u.kind === "dragonfly")
          marker
            .getElement()
            .setAttribute("aria-label", `Dead dragonfly ${u.id}`);
        if (!u.alive && !marker.getElement().querySelector(".death-cross"))
          marker
            .getElement()
            .insertAdjacentHTML(
              "beforeend",
              '<svg class="death-cross" viewBox="0 0 24 24"><path d="m3 3 18 18M3 21 21 3" stroke="currentColor" stroke-width="3"/></svg>',
            );
        marker.getElement().classList.toggle("selected", u.hive === hive);
        if (u.kind === "queen") {
          const playable = u.alive && u.hive === hive;
          marker
            .getElement()
            .setAttribute(
              "aria-label",
              !u.alive
                ? `Dead queen of hive ${u.hive + 1}`
                : playable
                  ? `Pilot queen of hive ${u.hive + 1}`
                  : `Select hive ${u.hive + 1}`,
            );
          if (playable && !marker.getElement().querySelector(".queen-play"))
            marker
              .getElement()
              .insertAdjacentHTML(
                "beforeend",
                '<svg class="queen-play" viewBox="0 0 24 24" aria-hidden="true"><path d="m8 4 13 8-13 8z" fill="currentColor"/></svg>',
              );
          if (!playable)
            marker.getElement().querySelector(".queen-play")?.remove();
        }
      }
    }
    for (const r of game.resources) {
      const id = `resource-${r.id}`;
      seen.add(id);
      let marker = markers.current.get(id);
      if (!marker) {
        const el = document.createElement("button");
        el.className = "resource-pin";
        el.onclick = (e) => {
          e.stopPropagation();
          if (lessonRef.current || placeRef.current) return;
          if (pilotRef.current) {
            return;
          } else m.easeTo({ center: [r.pos.lon, r.pos.lat], zoom: 3.3 });
        };
        marker = new maplibregl.Marker({
          element: el,
          opacityWhenCovered: 0,
          subpixelPositioning: true,
        })
          .setLngLat([r.pos.lon, r.pos.lat])
          .addTo(m);
        markers.current.set(id, marker);
      }
      if (marker.getLngLat().lng !== r.pos.lon || marker.getLngLat().lat !== r.pos.lat)
        marker.setLngLat([r.pos.lon, r.pos.lat]);
      marker
        .getElement()
        .setAttribute(
          "aria-label",
          `Resource ${r.id}: ${Math.round(r.energy)} joules, ${Math.round(r.minerals)} kilograms`,
        );
      const el = marker.getElement(), kind = r.energy > 0 && r.minerals > 0 ? "mixed" : r.energy > 0 ? "energy" : "mineral";
      if (el.dataset.kind !== kind) {
        el.dataset.kind = kind;
        el.innerHTML = `${r.energy > 0 ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="m13 2-9 12h7l-1 8 10-12h-7z"/></svg>' : ''}${r.minerals > 0 ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="m12 2 9 7-3 11H6L3 9zM3 9h18M12 2 8 9l4 11 4-11z"/></svg>' : ''}<span></span>`;
      }
      const label = el.querySelector("span")!;
      label.hidden = !pilotId;
      if (pilotId) label.textContent = Math.round(distance(piloted!.pos, r.pos)).toLocaleString();
    }
    for (const [id, x] of markers.current)
      if (!seen.has(id)) {
        x.remove();
        markers.current.delete(id);
      }
    if (pilotId) {
      if (!world.current) world.current = new LabWorld(m);
      world.current.update(
        game,
        flight.current?.firstPerson ? pilotId : undefined,
      );
    }
  }, [game, hive, pilotId, ready, selectHive]);
  const enter = useCallback(
    async (id: string) => {
      const g = data.current,
        u = g?.units.find((u) => u.id === id);
      if (
        !g ||
        !u ||
        !u.alive ||
        !map.current ||
        flight.current?.body.id === id
      )
        return;
      const serial = ++transition.current;
      setBusy(true);
      setError("");
      try {
        if (flight.current) release.current = flight.current.exit();
        flight.current = null;
        await release.current;
        const t = await labTerrain(id);
        if (serial !== transition.current) return;
        world.current?.destroy();
        world.current = null;
        selectedRef.current = u.hive;
        setHive(u.hive);
        pilotRef.current = id;
        placeRef.current = null;
        setPlace(null);
        setMenu(false);
        targetRef.current = null;
    targetMarkRef.current=null;
        setTarget(null);
        const result = await labCommand(
          g.epoch,
          armedRef.current ? "tutorial" : "pilot",
          id,
          undefined,
          u.hive,
        );
        if (serial !== transition.current) {
          void labCommand(g.epoch, "hold", id, undefined, u.hive);
          return;
        }
        accept(result);
        const current = result.units.find((x) => x.id === id)!;
        flight.current = new LabFlight(
          map.current,
          t,
          current,
          g.epoch,
          (b) => {
            if (!data.current || data.current.epoch !== g.epoch) return;
            const next = {
              ...data.current,
              units: data.current.units.map((u) => (u.id === b.id ? b : u)),
            };
            accept(next);
          },
          strike,
          setCamera,
          (id, pose, now, time) => world.current?.render(now, id, pose, time),
          (e) => {
            if (data.current?.epoch !== g.epoch) return;
            if (e instanceof LabError && e.message.includes("World replaced")) {
              void labState().then(accept).catch((error) => setError(String(error)));
            } else setError(String(e));
          },
          armedRef.current,
        );
        lessonRef.current = armedRef.current;
        setLesson(armedRef.current);
        setMenu(armedRef.current);
        setCamera("follow");
        setPilotId(id);
      } catch (e) {
        setError(String(e));
        home();
      } finally {
        if (serial === transition.current) setBusy(false);
      }
    },
    [accept, home, strike],
  );
  const enterGod = useCallback(async()=>{
    const g=data.current,m=map.current;if(!g||!m)return;
    const serial=++transition.current;setBusy(true);setError("");
    try {
      if(flight.current)await flight.current.exit();flight.current=null;
      const master=await labState();if(serial!==transition.current)return;
      world.current?.destroy();world.current=null;
      const c=m.getCenter(),anchor=master.units.find(u=>u.alive&&u.kind==="queen"&&u.hive===selectedRef.current);
      const body=godBody(anchor?{...anchor.pos,alt:anchor.pos.alt+500}:{lat:c.lat,lon:c.lng,alt:3000},master.time);
      if(armedRef.current)await labCommand(g.epoch,"tutorial_god");
      pilotRef.current=GOD_ID;setGod(body);setPilotId(GOD_ID);setMenu(false);setCamera("follow");
      selectedRef.current=null;setHive(null);accept(master);
      const terrain={lat:c.lat,lon:c.lng,step:250,size:2,heights:[0,0,0,0]};
      flight.current=new LabFlight(m,terrain,body,g.epoch,b=>setGod(b),()=>{},setCamera,
        (id,pose,now,time)=>world.current?.render(now,id,pose,time),e=>setError(String(e)),armedRef.current,true);
      lessonRef.current=armedRef.current;setLesson(armedRef.current);
    }catch(e){setError(String(e));home();}finally{if(serial===transition.current)setBusy(false);}
  },[accept,home]);
  const enterRef = useRef(enter);
  enterRef.current = enter;
  const chooseAdd = (kind: "queen" | "dragonfly" | "energy" | "mineral") => {
    if (lessonRef.current) return;
    if ((kind==="energy"||kind==="mineral") && feedCount.current>1 && data.current) {
      setBusy(true);void feedLab(data.current.epoch,feedCount.current,kind==="energy"?dropAmount.current.energy*1e9:0,kind==="mineral"?dropAmount.current.mineral:0).then(accept).catch(e=>setError(String(e))).finally(()=>setBusy(false));return;
    }
    const u = data.current?.units.find((u) => u.id === pilotRef.current);
    if (u) {
      const p = offset(
        u.pos,
        Math.sin(u.heading) * 140,
        Math.cos(u.heading) * 140,
      );
      void add(kind, p);
    } else {
      placeRef.current = kind;
      setPlace(kind);
      setMenu(false);
    }
  };
  const finishTutorial = async () => {
    const g = data.current;
    if (!g) return;
    try {
      const result = await labCommand(
        g.epoch,
        "tutorial_end",
        pilotRef.current ?? undefined,
        undefined,
        selectedRef.current ?? undefined,
      );
      accept(result);
      lessonRef.current = false;
      setLesson(false);
      armedRef.current = false;
      setArmed(false);
      localStorage.setItem("hive-tutorial", "0");
      setMenu(false);
      flight.current?.setTeaching(false);
    } catch (e) {
      setError(String(e));
    }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        lessonRef.current ||
        e.repeat ||
        e.ctrlKey ||
        e.metaKey ||
        e.altKey ||
        (e.target instanceof HTMLElement && e.target.matches("input,textarea"))
      )
        return;
      const u = data.current?.units.find(
        (u) => u.id === pilotRef.current && u.alive,
      );
      const action = (
        { KeyT: "scan", KeyG: "collect", KeyB: "deliver",KeyM:"mark" } as Record<
          string,
          string
        >
      )[e.code];
      if (u?.kind === "dragonfly" && action) {
        e.preventDefault();
        void send(action, u.id, undefined, u.hive);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [send]);
  useEffect(()=>{if(targetMarkRef.current!==null && game && !game.marks.some(m=>m.id===targetMarkRef.current)){targetMarkRef.current=null;targetRef.current=null;setTarget(null);}},[game]);
  const q = game?.units.filter((u) => u.kind === "queen" || u.founder) ?? [],
    members = game?.units.filter((u) => u.hive === hive) ?? [],
    pilot = pilotId===GOD_ID?god:game?.units.find((u) => u.id === pilotId),
    loading = busy || !ready || game?.loading;
  const allActive=game?.all_hives_active??false;
  return (
    <LessonContext.Provider value={lesson}>
      <main
        className={`lab ${pilotId ? "piloting" : "global"} ${place ? "placing" : ""}`}
        onPointerDown={unlockSound}
      >
        <div className="lab-map" ref={host} />
        {!pilotId && (
          <button
            className="tutorial-toggle"
            role="switch"
            aria-checked={armed}
            onClick={() => {
              const next = !armedRef.current;
              armedRef.current = next;
              setArmed(next);
              localStorage.setItem("hive-tutorial", next ? "1" : "0");
            }}
          >
            <span>Tutorial</span>
            <i />
          </button>
        )}

        <details className="source-credit">
          <summary aria-label="Map source attribution">
            <svg
              viewBox="0 0 24 24"
              width="18"
              height="18"
              fill="none"
              stroke="currentColor"
              aria-hidden="true"
            >
              <circle cx="12" cy="12" r="9" />
              <path d="M12 10v7M12 6v1" />
            </svg>
          </summary>
          <div>
            Terrain:{" "}
            <a href="https://registry.opendata.aws/terrain-tiles/">
              Mapzen / AWS
            </a>{" "}
            · <a href="https://openfreemap.org/">OpenFreeMap</a> · ©{" "}
            <a href="https://www.openstreetmap.org/copyright">
              OpenStreetMap contributors
            </a>
          </div>
        </details>
        <nav className="corner-controls" aria-label="View controls">
          <Icon
            icon="globe"
            label="Global home"
            onClick={home}
            active={!pilotId}
          />
          <AutonomySwitch active={allActive} label={allActive?'Deactivate all hives':'Activate all hives'}
            disabled={!game?.units.some(u=>u.alive)}
            onClick={()=>void send(allActive?'ai_disable_all':'ai_enable_all')}/>
          {!pilotId && <Icon icon="god" label="Pilot God craft" onClick={()=>void enterGod()}/>}
          {!pilotId && <Icon icon="reset" label="Configure starting world" active={startOpen} onClick={()=>setStartOpen(!startOpen)}/>}
          {!pilotId && (
            <Icon
              icon="shield"
              label={hollows === "hollows" ? "Return to terrain globe" : "Show H-500 hide spots"}
              onClick={toggleHollows}
              active={hollows === "hollows"}
            />
          )}
          {pilotId && (
            <>
              <Icon
                icon="camera"
                label="Switch first person and chase camera"
                onClick={() => flight.current?.toggleCamera()}
                active={camera === "firstperson"}
              />
              <Icon
                icon={camera === "lock" ? "lock" : "unlock"}
                label="Lock camera world direction"
                onClick={() => flight.current?.toggleLock()}
                active={camera === "lock"}
              />
            </>
          )}
        </nav>
        {!pilotId && startOpen && <StartPanel busy={busy} onClose={()=>setStartOpen(false)} onApply={async config=>{
          setBusy(true);setError("");
          try {const next=await resetLab(config);selectedRef.current=null;setHive(null);accept(next);setStartOpen(false);}
          catch(e){setError(String(e));}finally{setBusy(false);}
        }}/>}
        {!pilotId && (
          <nav className="hive-orbits" aria-label="Hives">
            {q.map((u) => (
              <button
                key={u.id}
                className={`hive-orbit ${hive === u.hive ? "active" : ""} ${u.alive ? "" : "dead"} ${(game?.ai_hives??[]).includes(u.hive)?'autonomous':''}`}
                style={{ color: colour(u.hive) }}
                onClick={() => selectHive(u.hive)}
                aria-label={`Select hive ${u.hive + 1}`}
              >
                <HiveEmblem hive={u.hive} lineage={u.lineage} size={40}/>
                <i className="hive-status" aria-hidden="true"/>
                {!u.alive&&<span className="hive-dead"><Symbol name="dead" size={14}/></span>}
              </button>
            ))}
          </nav>
        )}
        <div className={`spawn-controls ${pilotId&&!lesson?"flight-editor-hidden":""}`}>
          <Icon
            icon={place ? "close" : "plus"}
            label={place ? "Cancel placement" : "Add a member"}
            onClick={() => {
              if (place) {
                placeRef.current = null;
                setPlace(null);
              } else setMenu(!menu);
            }}
            active={menu || !!place}
          />
          {menu && (
            <div className="spawn-menu">
              <div className="drop-control"><Symbol name="collect"/><input type="number" min="1" max="1000" defaultValue="1" aria-label="Resource drop count; one places a point, multiple scatter over land" onChange={e=>feedCount.current=Number(e.target.value)}/></div>
              <Icon
                icon="queen"
                label="Add queen"
                onClick={() => chooseAdd("queen")}
              />
              <div className="drop-control">
                <Icon
                  icon="energy"
                  label="Drop energy resource"
                  onClick={() => chooseAdd("energy")}
                />
                <input
                  type="number"
                  min="1"
                  max="10000000"
                  defaultValue="250"
                  aria-label="Energy drop in billions of joules"
                  onChange={(e) =>
                    (dropAmount.current.energy = Number(e.target.value))
                  }
                />
                <small>×10⁹</small>
              </div>
              <div className="drop-control">
                <Icon
                  icon="mineral"
                  label="Drop mineral resource"
                  onClick={() => chooseAdd("mineral")}
                />
                <input
                  type="number"
                  min="1"
                  max="1000000000"
                  defaultValue="1000"
                  aria-label="Mineral drop in kilograms"
                  onChange={(e) =>
                    (dropAmount.current.mineral = Number(e.target.value))
                  }
                />
              </div>
              <Icon
                icon="dragonfly"
                label="Add dragonfly to selected hive"
                onClick={() => chooseAdd("dragonfly")}
                disabled={hive === null}
              />
            </div>
          )}
          {place && (
            <span className="placement-glyph">
              <Symbol name={place} />
            </span>
          )}
        </div>
        {!pilotId && hive !== null && (
          <section
            className="member-dock"
            aria-label={`Hive ${hive + 1} members`}
            style={{ "--hive": colour(hive) } as CSSProperties}
          >
            <AutonomySwitch label={(game?.ai_hives??[]).includes(hive)?"Disable hive autonomy":"Enable hive autonomy"}
              active={(game?.ai_hives??[]).includes(hive)}
              hive={hive} lineage={members[0]?.lineage}
              onClick={()=>void send((game?.ai_hives??[]).includes(hive)?"ai_disable":"ai_enable",undefined,undefined,hive)}/>
            {members.map((u) => (
              <button
                key={u.id}
                className={`member-card ${u.kind === "queen" ? "queen-card" : ""} ${u.alive ? "" : "dead"}`}
                aria-label={`${u.alive ? "Pilot" : "Dead"} ${u.kind} ${u.id}`}
                disabled={!u.alive}
                onClick={() => void enter(u.id)}
              >
                <span className="member-sigil">
                  <Symbol name={u.kind} size={28} />
                  <Symbol name={u.alive ? "pilot" : "dead"} size={12} />
                </span>
                <span
                  className="coordinates"
                  aria-label={`Latitude ${u.pos.lat.toFixed(4)}, longitude ${u.pos.lon.toFixed(4)}, altitude ${Math.round(u.pos.alt)} metres`}
                >
                  <span>{u.pos.lat.toFixed(3)}°</span>
                  <span>{u.pos.lon.toFixed(3)}°</span>
                  <span className="coord-alt">
                    <Symbol name="altitude" size={10} />
                    {Math.round(u.pos.alt).toLocaleString()}
                  </span>
                </span>
              </button>
            ))}
          </section>
        )}
        {pilot && game && pilot.alive && (
          <>
            <GlobalRadar
              game={game}
              pilot={pilot}
              target={target}
              teaching={lesson}
              onUpdate={accept}
              onTarget={(m) => {
                if (lessonRef.current) return;
                const p={...m.pos};targetRef.current=p;targetMarkRef.current=m.id;setTarget(p);
              }}
            />
            <div
              className="flight-reticle"
              style={
                {
                  "--bank": `${-pilot.roll}rad`,
                  "--pitch": `${pilot.pitch * 35}px`,
                } as CSSProperties
              }
              aria-label="Flight attitude"
            >
              <span />
              <i />
              <b />
            </div>
            <section
              className="flight-instruments"
              style={{ "--hive": colour(pilot.hive) } as CSSProperties}
              aria-label="Flight instruments"
            >
              <div className="instrument">
                <span aria-label={`Altitude assistance ${flight.current?.assisted ? "enabled" : "disabled"}`}
                  style={{color: flight.current?.assisted ? colour(pilot.hive) : "#85929b"}}>
                  <Symbol name="altitude" size={15} />
                </span>
                <span
                  aria-label={`Terrain clearance ${Math.round(pilot.agl)} metres`}
                >
                  {pilot.pos.alt>20000 ? "—" : Math.max(0, Math.round(pilot.agl)).toLocaleString()}
                </span>
                <i
                  className={pilot.radar_visible ? "exposed-dot" : "hidden-dot"}
                  aria-label={
                    pilot.radar_visible
                      ? `Visible on global radar above ${Math.round(pilot.radar_altitude ?? 0)} metres above sea level`
                      : `Hidden below radar altitude ${Math.round(pilot.radar_altitude ?? 0)} metres above sea level`
                  }
                />
              </div>
              <span
                className={`shelter-indicator ${pilot.nuclear_sheltered ? "covered" : "exposed"}`}
                aria-label={pilot.kind==="god"?"Invincible God craft":`Nuclear protection: ${pilot.nuclear_sheltered ? "general blast cover" : "exposed"}`}
              >
                <Symbol name="shield" size={18} />
              </span>
              <div className="instrument">
                <Symbol name="speed" size={15} />
                <span
                  aria-label={`Speed ${Math.hypot(pilot.v.x,pilot.v.y,pilot.v.z).toFixed(1)} metres per second`}
                >
                  {Math.hypot(pilot.v.x,pilot.v.y,pilot.v.z).toFixed(1)}
                </span>
              </div>
              {pilot.kind!=="god" && <div
                className="cargo-instrument"
                aria-label={`Cargo ${pilot.cargo_energy} joules, ${pilot.cargo_minerals} kilograms; stored minerals ${pilot.minerals} kilograms`}
              >
                <Symbol name="collect" size={18} />
                {pilot.kind==="queen" && <span aria-label={`Body mass ${pilot.dry_mass} kilograms`}>{(pilot.dry_mass/1000).toFixed(0)}</span>}
                <Symbol name="energy" size={12} />
                <span>{(pilot.cargo_energy / 1e9).toFixed(0)}</span>
                <Symbol name="mineral" size={12} />
                <span>
                  {Math.round(
                    pilot.kind === "queen"
                      ? pilot.minerals
                      : pilot.cargo_minerals,
                  )}
                </span>
              </div>}
              {pilot.kind==="god"?<span aria-label="Invincible God craft"><Symbol name="god"/><b>∞</b></span>:<Meter
                ratio={pilot.energy / pilot.capacity}
                icon="energy"
                label={`Energy ${Math.round(pilot.energy)} joules, draw ${Math.round(pilot.power)} watts`}
              />}
            </section>
            {pilot.kind!=="god" && <div
              className="weapon-controls"
              style={{ "--hive": colour(pilot.hive) } as CSSProperties}
            >
              {pilot.founder && <>
                <Icon icon={pilot.grounded?"lift":"land"} label={pilot.grounded?"Lift founder":"Land founder"} disabled={pilot.construction_remaining>0} onClick={()=>void send(pilot.grounded?"lift_founder":"land",pilot.id,undefined,pilot.hive)}/>
                <Icon icon="queen" label="Found queen" active={pilot.construction_remaining>0} disabled={!pilot.grounded||pilot.construction_remaining>0} onClick={()=>void send("found",pilot.id,undefined,pilot.hive)}/>
                <Icon icon="energy" label="Refuel founder from cargo" onClick={()=>void send("refuel_self",pilot.id,undefined,pilot.hive)}/>
              </>}
              {pilot.kind === "queen" ? (
                <>
                  <Icon
                    icon={pilot.grounded ? "lift" : "land"}
                    label={pilot.grounded ? "Lift queen" : "Settle queen"}
                    onClick={() =>
                      void send(
                        pilot.grounded ? "lift" : "land",
                        pilot.id,
                        undefined,
                        pilot.hive,
                      )
                    }
                    disabled={["preparing", "rising", "landing"].includes(
                      pilot.mode,
                    )}
                  />
                  {pilot.grounded && <>
                    <Icon icon="collect" label="Collect nearby resource" onClick={()=>void send("collect",pilot.id,undefined,pilot.hive)}/>
                    <Icon icon="queen" label="Grow queen" active={pilot.growth_target>pilot.dry_mass} onClick={()=>void send("grow",pilot.id,undefined,pilot.hive)}/>
                  </>}
                  {(lesson || pilot.grounded) && <Icon icon="dragonfly" label="Reproduce dragonfly" active={pilot.brood_remaining>0} disabled={!pilot.grounded||pilot.brood_remaining>0||pilot.energy<game.balance.birth_energy||pilot.minerals<game.balance.birth_minerals} onClick={()=>void send("reproduce",pilot.id,undefined,pilot.hive)}/>}
                  {lesson && (
                    <Icon
                      icon={pilot.grounded ? "land" : "lift"}
                      label={pilot.grounded ? "Settle queen" : "Lift queen"}
                      onClick={() => {}}
                    />
                  )}
                  <button
                    className={`weapon-button ${pilot.charge ? "charging" : ""}`}
                    aria-label={
                      pilot.charge
                        ? "Cancel queen strike charge"
                        : "Charge queen strike at selected dragonfly mark"
                    }
                    onClick={strike}
                    disabled={
                      !lesson &&
                      !pilot.charge &&
                      (!target ||
                        !pilot.radar_visible ||
                        pilot.energy < game.balance.strike_energy ||
                        pilot.cooldown > 0)
                    }
                  >
                    <svg
                      viewBox="0 0 64 64"
                      className="weapon-ring"
                      aria-hidden="true"
                    >
                      <circle cx="32" cy="32" r="29" />
                      <circle
                        cx="32"
                        cy="32"
                        r="29"
                        pathLength="100"
                        strokeDasharray={`${pilot.charge ? (1 - pilot.charge.remaining / 30) * 100 : pilot.cooldown ? 100 - (pilot.cooldown / 180) * 100 : 0} 100`}
                      />
                    </svg>
                    <Symbol
                      name={pilot.charge ? "close" : "strike"}
                      size={28}
                    />
                  </button>
                  {lesson && !pilot.charge && (
                    <Icon
                      icon="close"
                      label="Cancel queen strike charge"
                      onClick={() => {}}
                    />
                  )}
                </>
              ) : (
                <>
                  <Icon
                    icon="radar"
                    label="Scan for resources"
                    active={pilot.scan_until > game.time}
                    onClick={() =>
                      void send("scan", pilot.id, undefined, pilot.hive)
                    }
                  />
                  <Icon icon="mark" label="Mark dragonfly location" onClick={()=>void send("mark",pilot.id,undefined,pilot.hive)}/>
                  {(lesson || game.resources.some(r=>distance(r.pos,pilot.pos)<120)) && <Icon
                    icon="collect"
                    label="Collect nearby resource"
                    onClick={() =>
                      void send("collect", pilot.id, undefined, pilot.hive)
                    }
                  />}
                  {(lesson || game.units.some(q=>q.alive&&q.kind==="queen"&&q.hive===pilot.hive&&distance(q.pos,pilot.pos)<300)) && <Icon
                    icon="deliver"
                    label="Deliver cargo and refuel"
                    onClick={() =>
                      void send("deliver", pilot.id, undefined, pilot.hive)
                    }
                  />}
                  <button
                    className="weapon-button spit-button"
                    aria-label="Fire dragonfly spit"
                    onPointerDown={() => {
                      if (lessonRef.current) return;
                      flight.current?.fireSpit();
                    }}
                  >
                    <svg
                      viewBox="0 0 40 40"
                      fill="none"
                      stroke="currentColor"
                      aria-hidden="true"
                    >
                      <circle cx="27" cy="12" r="5" />
                      <path d="m7 30 11-11M4 23l10-10M15 34l10-10" />
                    </svg>
                  </button>
                </>
              )}
            </div>}
            {game.missiles.some((s) =>
              game.units.some(
                (u) =>
                  u.hive === pilot.hive && distance(u.pos, s.target) < game.balance.blast_radius+100,
              ),
            ) && (
              <div
                className="incoming-symbol"
                aria-label="Incoming nuclear strike"
              >
                <Symbol name="warning" size={28} />
              </div>
            )}
          </>
        )}
        {lesson && pilot && (
          <Tutorial
            queen={pilot.kind === "queen"}
            god={pilot.kind === "god"}
            finish={() => void finishTutorial()}
          />
        )}
        {pilotId && (!pilot || !pilot.alive) && (
          <button
            className="lost-member"
            aria-label="Member destroyed, return to globe"
            onClick={home}
          >
            <Symbol name={pilot?.kind ?? "dragonfly"} size={40} />
            <Symbol name="close" size={20} />
          </button>
        )}
        {loading && (
          <div
            className="loading-orbit"
            role="status"
            aria-label="Loading terrain"
          >
            <span />
          </div>
        )}
        {error && (
          <button
            className="error-symbol"
            aria-label={error}
            onClick={() => setError("")}
          >
            <Symbol name="warning" />
          </button>
        )}
        {game?.bursts.some((b) => b.kind === "nuclear" && b.age < 0.8) && (
          <div className="nuclear-flash" />
        )}
      </main>
    </LessonContext.Provider>
  );
}
