//! Wayland idle notifications that respect compositor inhibitors.

use super::IdleState;
use smithay_client_toolkit::reexports::{
    calloop::EventLoop, calloop_wayland_source::WaylandSource,
};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for IdleState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for IdleState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for IdleState {
    fn event(
        _: &mut Self,
        _: &ext_idle_notifier_v1::ExtIdleNotifierV1,
        _: ext_idle_notifier_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, usize> for IdleState {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled
                if !state.inhibited() && !state.restart_pending =>
            {
                state.rules[*index].idled()
            }
            ext_idle_notification_v1::Event::Resumed => state.rules[*index].resumed(),
            _ => {}
        }
    }
}
pub fn run(event_loop: &mut EventLoop<'_, IdleState>, mut state: IdleState) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("Wayland connection: {e}"))?;
    let (globals, queue) =
        registry_queue_init::<IdleState>(&conn).map_err(|e| format!("Wayland registry: {e}"))?;
    let qh = queue.handle();
    let notifier: ext_idle_notifier_v1::ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("ext-idle-notify-v1 unavailable: {e}"))?;
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("Wayland seat unavailable: {e}"))?;
    let mut notifications: Vec<_> = state
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| notifier.get_idle_notification(rule.timeout_ms, &seat, &qh, index))
        .collect();
    WaylandSource::new(conn, queue)
        .insert(event_loop.handle())
        .map_err(|error| format!("Wayland event source: {error}"))?;
    loop {
        event_loop
            .dispatch(None, &mut state)
            .map_err(|error| format!("Wayland event dispatch: {error}"))?;
        if state.restart_pending && !state.inhibited() {
            // Keep notifications for rules that fired so their Resumed event
            // can still run the corresponding resume command.
            for (index, (rule, notification)) in
                state.rules.iter().zip(&mut notifications).enumerate()
            {
                if !rule.fired() {
                    notification.destroy();
                    *notification =
                        notifier.get_idle_notification(rule.timeout_ms, &seat, &qh, index);
                }
            }
            state.restart_pending = false;
        }
    }
}
