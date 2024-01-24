use serde::Serialize;

use super::*;

#[derive(Debug, Clone)]
pub struct CarnotState {
    pub(crate) node_id: NodeId,
    pub(crate) current_view: View,
    pub(crate) highest_voted_view: View,
    pub(crate) local_high_qc: StandardQc,
    pub(crate) safe_blocks: HashMap<BlockId, Block>,
    pub(crate) last_view_timeout_qc: Option<TimeoutQc>,
    pub(crate) latest_committed_block: Block,
    pub(crate) latest_committed_view: View,
    pub(crate) root_committee: Committee,
    pub(crate) parent_committee: Option<Committee>,
    pub(crate) child_committees: Vec<Committee>,
    pub(crate) committed_blocks: Vec<BlockId>,
    pub(super) step_duration: Duration,

    /// Step id for this state
    pub(super) step_id: usize,
    /// does not serialize this field, this field is used to check
    /// how to serialize other fields because csv format does not support
    /// nested map or struct, we have to do some customize.
    pub(super) format: SubscriberFormat,
}

impl CarnotState {
    pub(super) fn new<O: Overlay>(
        step_id: usize,
        step_duration: Duration,
        fmt: SubscriberFormat,
        engine: &Carnot<O>,
    ) -> Self {
        let mut this = Self::from(engine);
        this.step_id = step_id;
        this.step_duration = step_duration;
        this.format = fmt;
        this
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum CarnotRecord {
    Runtime(Runtime),
    Settings(Box<SimulationSettings>),
    Data(Vec<Box<CarnotState>>),
}

impl From<Runtime> for CarnotRecord {
    fn from(value: Runtime) -> Self {
        Self::Runtime(value)
    }
}

impl From<SimulationSettings> for CarnotRecord {
    fn from(value: SimulationSettings) -> Self {
        Self::Settings(Box::new(value))
    }
}

impl Record for CarnotRecord {
    type Data = CarnotState;

    fn record_type(&self) -> RecordType {
        match self {
            CarnotRecord::Runtime(_) => RecordType::Meta,
            CarnotRecord::Settings(_) => RecordType::Settings,
            CarnotRecord::Data(_) => RecordType::Data,
        }
    }

    fn data(&self) -> Vec<&CarnotState> {
        match self {
            CarnotRecord::Data(d) => d.iter().map(AsRef::as_ref).collect(),
            _ => vec![],
        }
    }
}

impl<S, T: Clone + Serialize + 'static> TryFrom<&SimulationState<S, T>> for CarnotRecord {
    type Error = anyhow::Error;

    fn try_from(state: &SimulationState<S, T>) -> Result<Self, Self::Error> {
        let Ok(states) = state
            .nodes
            .read()
            .iter()
            .map(|n| Box::<dyn Any + 'static>::downcast(Box::new(n.state().clone())))
            .collect::<Result<Vec<_>, _>>()
        else {
            return Err(anyhow::anyhow!("use carnot record on other node"));
        };
        Ok(Self::Data(states))
    }
}

impl serde::Serialize for CarnotState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(rs) = RECORD_SETTINGS.get() {
            let keys = rs
                .iter()
                .filter_map(|(k, v)| {
                    if serde_util::CARNOT_RECORD_KEYS.contains(&k.trim()) && *v {
                        Some(k)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            match self.format {
                SubscriberFormat::Json => serde_util::CarnotStateJsonSerializer::default()
                    .serialize_state(keys, self, serializer),
                SubscriberFormat::Csv => serde_util::CarnotStateCsvSerializer::default()
                    .serialize_state(keys, self, serializer),
                SubscriberFormat::Parquet => unreachable!(),
            }
        } else {
            serializer.serialize_none()
        }
    }
}

impl CarnotState {
    const fn keys() -> &'static [&'static str] {
        serde_util::CARNOT_RECORD_KEYS
    }
}

impl<O: Overlay> From<&Carnot<O>> for CarnotState {
    fn from(value: &Carnot<O>) -> Self {
        let node_id = value.id();
        let current_view = value.current_view();
        Self {
            node_id,
            current_view,
            local_high_qc: value.high_qc(),
            parent_committee: value.parent_committee(),
            root_committee: value.root_committee(),
            child_committees: value.child_committees(),
            latest_committed_block: value.latest_committed_block(),
            latest_committed_view: value.latest_committed_view(),
            safe_blocks: value
                .blocks_in_view(current_view)
                .into_iter()
                .map(|b| (b.id, b))
                .collect(),
            last_view_timeout_qc: value.last_view_timeout_qc(),
            committed_blocks: value.latest_committed_blocks(None),
            highest_voted_view: Default::default(),
            step_duration: Default::default(),
            format: SubscriberFormat::Csv,
            step_id: 0,
        }
    }
}
