// Instants resolved by JavaScript Date at local wall times; see planning-workday-fixtures.json.
const FIXTURES: [WorkdayFixture; 5] = [
    WorkdayFixture { name: "Los_Angeles_2026_3_7", midnight: 1772870400000.0, now: 1772892000000.0, window: DayPlanWindow { start_at_ms: 1772902800000.0, end_at_ms: 1772935200000.0 }, legacy_differs: false },
    WorkdayFixture { name: "Los_Angeles_2026_3_8", midnight: 1772956800000.0, now: 1772974800000.0, window: DayPlanWindow { start_at_ms: 1772985600000.0, end_at_ms: 1773018000000.0 }, legacy_differs: true },
    WorkdayFixture { name: "Los_Angeles_2026_10_31", midnight: 1793430000000.0, now: 1793451600000.0, window: DayPlanWindow { start_at_ms: 1793462400000.0, end_at_ms: 1793494800000.0 }, legacy_differs: false },
    WorkdayFixture { name: "Los_Angeles_2026_11_1", midnight: 1793516400000.0, now: 1793541600000.0, window: DayPlanWindow { start_at_ms: 1793552400000.0, end_at_ms: 1793584800000.0 }, legacy_differs: true },
    WorkdayFixture { name: "UTC_2026_3_8", midnight: 1772928000000.0, now: 1772949600000.0, window: DayPlanWindow { start_at_ms: 1772960400000.0, end_at_ms: 1772992800000.0 }, legacy_differs: false },
];
