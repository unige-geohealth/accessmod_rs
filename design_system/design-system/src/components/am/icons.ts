// AM5 Font Awesome icons → Lucide. One semantic name per meaning; use these,
// not Lucide directly, so the mapping stays in one place (docs/icons.md).
import {
  Archive,
  Boxes,
  ChartColumn,
  CircleHelp,
  CircleMinus,
  CirclePlay,
  CirclePlus,
  Clock,
  Cog,
  Download,
  ExternalLink,
  FolderOpen,
  Funnel,
  Grid3x3,
  Hospital,
  House,
  Info,
  Languages,
  List,
  LogIn,
  LogOut,
  Map as MapIcon,
  Minus,
  MapPin,
  Menu,
  Pencil,
  Play,
  Plus,
  Redo2,
  RefreshCw,
  RotateCcw,
  SkipForward,
  SlidersHorizontal,
  Square,
  SquareCheck,
  Star,
  Table,
  Trash2,
  TriangleAlert,
  Undo2,
  Users,
  GripVertical,
  Spline,
  WandSparkles,
  Wrench,
  X,
} from "lucide-react";

export const icons = {
  // sidebar sections
  project: MapPin, // fa-map-marker
  data: FolderOpen, // fa-folder-open
  analysis: Boxes, // fa-cubes
  toolbox: Cog, // fa-cogs
  logs: Archive, // fa-archive
  settings: SlidersHorizontal, // fa-sliders-h
  about: Info, // fa-info-circle
  menu: Menu, // fa-bars (header toggle)

  // analysis modules
  accessibility: Clock, // fa-clock
  coverage: Hospital, // fa-hospital
  referral: Table, // fa-table
  zonal: ChartColumn, // fa-chart-bar
  scalingUp: Plus, // fa-plus
  optimization: Star, // fa-star

  // toolbox tools
  map: MapIcon, // fa-map
  mergeLandCover: List, // fa-list
  adjustPopulation: Users, // fa-users
  raster: Grid3x3, // fa-th (map tools › Raster)

  // section cards
  dataInput: LogIn, // fa-right-to-bracket
  output: LogOut, // fa-sign-out-alt / fa-right-from-bracket (output summary)
  referralFrom: Play, // fa-play ("From" facilities, Referral)
  referralTo: Square, // fa-stop ("To" facilities, Referral)
  analysisSettings: Wrench, // fa-wrench
  validation: SquareCheck, // fa-square-check
  tables: Table, // fa-table
  auto: WandSparkles, // fa-wand-magic

  // project / data actions
  open: CirclePlay, // fa-play-circle
  add: CirclePlus, // fa-plus-circle
  remove: CircleMinus, // fa-minus-circle
  delete: Trash2, // fa-trash-alt
  export: ExternalLink, // fa-external-link-alt
  language: Languages, // fa-language
  filter: Funnel, // fa-filter
  rename: RefreshCw, // fa-sync
  archive: Download, // fa-download
  reset: RotateCcw, // fa-undo
  edit: Pencil, // fa-pencil
  home: House, // fa-home (map: back to project extent)
  zoomIn: Plus, // leaflet +
  zoomOut: Minus, // leaflet −
  next: SkipForward, // fa-step-forward (map edit: next facility)
  undo: Undo2, // fa-undo (map edit)
  redo: Redo2, // (new: AM5 had undo only)
  close: X, // fa-times

  // stack items (Merge land cover)
  drag: GripVertical, // fa-sort (drag handle)
  stackRaster: Grid3x3, // fa-th (raster item)
  stackVector: Spline, // line / polygon item
  download: Download, // export bar

  // messages
  info: Info, // fa-circle-info
  warning: TriangleAlert, // fa-triangle-exclamation
  help: CircleHelp, // fa-question-circle
} as const;

export type IconName = keyof typeof icons;
