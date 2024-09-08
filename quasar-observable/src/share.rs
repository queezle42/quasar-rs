use std::sync::Weak;
use tokio::sync::Notify;

use super::*;

pub struct Share<O>
where
    O: Observable,
{
    // Primary mutex. Will be locked while handling upstream observer messages,
    // e.g. while dispatching them to downstream observers.
    //
    // While holding this mutex, a thread might only hold the `request_state`
    // mutex for internal state management. Holding both mutexes and _then_
    // calling a downstream observer is unsafe and will lead to a deadlock.
    state: Mutex<State<O>>,

    // Downstream mutex - may be locked temporarily by downstream observers at
    // any time. When a thread holds this mutex, it must never try to wait for
    // the `state` mutex.
    //
    // A thread must not call a downstream observer while holding this mutex.
    request_state: Arc<Mutex<RequestState<O>>>,

    request_notify: Arc<Notify>,
}

impl<O> Drop for Share<O>
where
    O: Observable,
{
    fn drop(&mut self) {
        // stop management task
        self.request_notify.notify_one();
    }
}

struct State<O>
where
    O: Observable,
{
    downstreams: BTreeMap<u64, Downstream<O>>,
    // invariant: is always Some except when updating the ObserverState
    observer_state: Option<ObserverState<O::T, O::E, O::W>>,
    active_selector: Selector,
}

struct RequestState<O>
where
    O: Observable,
{
    detach_requests: Vec<u64>,
    configure_requests: BTreeMap<u64, Selector>,
    attach_requests: Vec<(u64, Downstream<O>)>,
    next_downstream_id: u64,
}

struct Downstream<O>
where
    O: Observable,
{
    selector: Selector,
    observer: ObserverBox<O::T, O::E, O::W, O::U>,
}

impl<O> Share<O>
where
    O: Observable + Send,
    O::T: Clone,
    O::E: Clone,
    O::W: Clone,
{
    pub fn new(observable: O) -> Arc<Share<O>> {
        let share_state = Mutex::new(State {
            downstreams: BTreeMap::new(),
            observer_state: Some(ObserverState::new()),
            active_selector: Selector::default(),
        });

        let request_state = Arc::new(Mutex::new(RequestState {
            detach_requests: Vec::new(),
            configure_requests: BTreeMap::new(),
            attach_requests: Vec::new(),
            next_downstream_id: 0,
        }));

        let share = Arc::new(Share {
            state: share_state,
            request_state,
            request_notify: Arc::new(Notify::new()),
        });

        tokio::task::spawn(share.clone().attach_and_manage_requests(observable));

        share
    }

    async fn attach_and_manage_requests(self: Arc<Share<O>>, observable: O) {
        let request_notify = self.request_notify.clone();
        let weak = Arc::downgrade(&self);
        drop(self);

        // wait for first request
        request_notify.notified().await;

        if let Some(this) = weak.clone().upgrade() {
            let selector = {
                let mut state = this.state.lock().unwrap();
                Self::get_next_selector(&mut state).unwrap_or_default()
            };
            // NOTE state lock not held while attaching
            let attached = observable.attach(ShareObserver(weak.clone()), selector);
            drop(this);
            Self::manage_requests(weak, request_notify, attached).await;
        }
    }

    async fn manage_requests(
        this: Weak<Share<O>>,
        request_notify: Arc<Notify>,
        mut attached: AttachedObservable,
    ) {
        loop {
            // wait for next request
            request_notify.notified().await;

            if let Some(mut this) = this.clone().upgrade() {
                this.handle_request(&mut attached);
            } else {
                // Share was dropped
                attached.detach();
                return;
            }
        }
    }

    fn handle_request(self: &mut Arc<Share<O>>, attached: &mut AttachedObservable) {
        let selector = {
            // NOTE this is the allowed mutex order: state then request_state
            let mut state = self.state.lock().unwrap();
            let mut request_state = self.request_state.lock().unwrap();

            for id in take(&mut request_state.detach_requests) {
                if let Some(downstream) = state.downstreams.remove(&id) {
                    downstream.observer.done();
                }
            }

            for (id, selector) in take(&mut request_state.configure_requests) {
                if let Some(downstream) = state.downstreams.get_mut(&id) {
                    downstream.selector = selector;
                }
            }

            for (id, downstream) in take(&mut request_state.attach_requests) {
                state.downstreams.insert(id, downstream);
            }

            Self::get_next_selector(&mut state)
        };

        if let Some(selector) = selector {
            // NOTE state lock not held before sending upstream request
            attached.configure(selector);
        }
    }

    fn get_next_selector(state: &mut State<O>) -> Option<Selector> {
        let selectors = state.downstreams.values().map(|d| d.selector.clone());
        let selector = Selector::combine(selectors);
        if state.active_selector != selector {
            state.active_selector = selector.clone();
            Some(selector)
        } else {
            None
        }
    }

    fn attach_downstream(
        self: Arc<Share<O>>,
        observer: ObserverBox<O::T, O::E, O::W, O::U>,
        selector: Selector,
    ) -> AttachedObservable {
        let mut request_state = self.request_state.lock().unwrap();
        let downstream_id = request_state.next_downstream_id;
        request_state.next_downstream_id += 1;

        let downstream = Downstream { selector, observer };
        request_state
            .attach_requests
            .push((downstream_id, downstream));

        let request_state_1 = self.request_state.clone();
        let request_state_2 = self.request_state.clone();
        let request_notify_1 = self.request_notify.clone();
        let request_notify_2 = self.request_notify.clone();

        AttachedObservable::new(
            move || Self::detach_downstream(request_state_1, request_notify_1, downstream_id),
            move |selector| {
                Self::configure_downstream(
                    &request_state_2,
                    &request_notify_2,
                    downstream_id,
                    selector,
                )
            },
        )

        // TODO signal reconfigure thread
    }

    fn detach_downstream(
        request_state: Arc<Mutex<RequestState<O>>>,
        request_notify: Arc<Notify>,
        downstream_id: u64,
    ) {
        let mut request_state = request_state.lock().unwrap();
        request_state.detach_requests.push(downstream_id);
        request_notify.notify_one();
    }

    fn configure_downstream(
        request_state: &Mutex<RequestState<O>>,
        request_notify: &Notify,
        downstream_id: u64,
        selector: Selector,
    ) {
        let mut request_state = request_state.lock().unwrap();
        request_state
            .configure_requests
            .insert(downstream_id, selector);
        request_notify.notify_one();
    }

    fn reconfigure(self: &Arc<Share<O>>, state: &mut State<O>) {
        // let selectors = state.downstreams.values().map(|d| d.selector.clone());
        // let selector = Selector::combine(selectors);
        //
        // self.attached.configure(selector);
    }
}

impl<O> Downstream<O> where O: Observable {}

impl<O> Observable for Arc<Share<O>>
where
    O: Observable + Send,
    O::T: Clone,
    O::E: Clone,
    O::W: Clone,
{
    type T = O::T;
    type E = O::E;
    type W = O::W;
    type U = O::U;

    fn attach<P: Observer<O::T, O::E, O::W, O::U>>(
        self,
        observer: P,
        selector: Selector,
    ) -> AttachedObservable {
        self.attach_downstream(observer.into_box(), selector)
    }

    fn attach_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        selector: Selector,
    ) -> AttachedObservable {
        self.attach(observer, selector)
    }
}

impl<O> SharedObservable for Arc<self::share::Share<O>>
where
    O: Observable + Send,
    O::T: Clone,
    O::E: Clone,
    O::W: Clone,
{
    fn clone_box(&self) -> SharedObservableBox<Self::T, Self::E, Self::W, Self::U> {
        SharedObservableBox::new(self.clone())
    }
}

struct ShareObserver<O>(Weak<Share<O>>)
where
    O: Observable;

impl<O> Observer<O::T, O::E, O::W, O::U> for ShareObserver<O>
where
    O: Observable + Send,
    O::T: Clone,
    O::E: Clone,
    O::W: Clone,
{
    fn set_changing(&mut self, clear_cache: bool) {
        if let Some(this) = self.0.upgrade() {
            let mut state = this.state.lock().unwrap();
            state.observer_state = Some(
                take(&mut state.observer_state)
                    .unwrap()
                    .set_changing(clear_cache),
            );
            todo!()
        }
    }
    fn set_waiting(&mut self, clear_cache: bool, marker: O::W) {
        if let Some(this) = self.0.upgrade() {
            let mut state = this.state.lock().unwrap();
            state.observer_state = Some(
                take(&mut state.observer_state)
                    .unwrap()
                    .set_waiting(clear_cache, marker),
            );
            todo!()
        }
    }
    fn set_live(
        &mut self,
        content: Option<Result<O::T, O::E>>,
    ) -> Box<dyn Future<Output = ()> + Sync> {
        if let Some(this) = self.0.upgrade() {
            let mut state = this.state.lock().unwrap();
            state.observer_state = Some(take(&mut state.observer_state).unwrap().set_live(content));
            todo!()
        } else {
            Box::new(std::future::pending())
        }
    }
    fn update(&mut self, update: O::U) -> Box<dyn Future<Output = ()> + Sync> {
        if let Some(this) = self.0.upgrade() {
            let mut state = this.state.lock().unwrap();
            state.observer_state = Some(take(&mut state.observer_state).unwrap().update(update));
            todo!()
        } else {
            Box::new(std::future::pending())
        }
    }
}
