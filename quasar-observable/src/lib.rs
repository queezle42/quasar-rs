#![feature(trait_upcasting)]

use std::any::Any;
use std::boxed::Box;
use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;
use std::ops::FnMut;
use std::ops::FnOnce;
use std::sync::Arc;

pub trait Observable
where
    Self: Sized + 'static,
{
    type T: Send;
    type E: Send;

    #[must_use]
    fn attach<P: Observer<Self::T, Self::E> + 'static>(
        self,
        observer: P,
    ) -> impl FnOnce() -> (Self, P) + Send;

    fn retrieve(self) -> impl Future<Output = Result<Self::T, Self::E>> + Send {
        self::retrieve::retrieve(self)
    }

    fn share(self) -> share::Share<Self> {
        share::Share::new(self)
    }

    fn map<F>(self, f: F) -> self::map::Map<Self, F>
    where
        F: FnMut(Self::T) -> Self::T,
    {
        self::map::Map::new(self, f)
    }
}

pub trait Container {
    type Item;
    type Update;
    fn apply_update(self, update: Self::Update) -> Self;
    fn apply_update_mut(&mut self, update: Self::Update);
    fn merge_updates(current: Self::Update, next: Self::Update);
}

pub trait ContainerObservable: Observable
where
    Self::T: Container,
{
    fn attach_container<P: ContainerObserver<Self::T, Self::E> + 'static>(
        self,
        observer: P,
    ) -> impl FnOnce() -> (Self, P) + Send;
}

pub trait Observer<T, E>: Any + Send {
    fn set_changing(&mut self, clear_cache: bool);
    fn set_live(&mut self, content: Option<Result<T, E>>) -> Box<dyn Future<Output = ()> + Sync>;
}

pub trait ContainerObserver<T, E>: Observer<T, E>
where
    T: Container,
{
    fn update(&mut self, update: <T as Container>::Update) -> Box<dyn Future<Output = ()> + Sync>;
}

pub struct AttachedObserver(Option<Box<dyn FnOnce() + Sync + Send>>);
impl Drop for AttachedObserver {
    fn drop(&mut self) {
        let AttachedObserver(state) = self;
        if let Some(detach) = std::mem::take(state) {
            detach();
        }
    }
}

enum ObserverState<T, E> {
    Waiting(Option<Result<T, E>>),
    Live(Result<T, E>),
}

impl<T, E> ObserverState<T, E> {
    fn set_waiting(self, clear_cache: bool) -> ObserverState<T, E> {
        if clear_cache {
            ObserverState::Waiting(None)
        } else {
            match self {
                ObserverState::Waiting(cache) => ObserverState::Waiting(cache),
                ObserverState::Live(content) => ObserverState::Waiting(Some(content)),
            }
        }
    }

    fn set_live(self, content: Option<Result<T, E>>) -> ObserverState<T, E> {
        match content {
            Some(content) => ObserverState::Live(content),
            None => match self {
                ObserverState::Waiting(None) => panic!(),
                ObserverState::Waiting(Some(cache)) => ObserverState::Live(cache),
                ObserverState::Live(content) => ObserverState::Live(content),
            },
        }
    }
}

mod retrieve {
    use super::*;

    struct Retrieve<O>
    where
        O: Observable,
    {
        state: Option<ObserverState<O::T, O::E>>,
        tx: Option<tokio::sync::oneshot::Sender<Result<O::T, O::E>>>,
    }

    pub fn retrieve<O>(observable: O) -> impl Future<Output = Result<O::T, O::E>> + Send
    where
        O: Observable,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let retrieve = Retrieve::<O> {
            state: Some(ObserverState::Waiting(None)),
            tx: Some(tx),
        };
        let attached = observable.attach(retrieve);

        async {
            let result = rx.await;
            drop(attached);

            match result {
                Ok(result) => result,
                Err(_) => {
                    // this should not happen because this task holds a reference to
                    // the observable
                    panic!("observer dropped without receiving a value")
                }
            }
        }
    }

    impl<O> Observer<O::T, O::E> for Retrieve<O>
    where
        O: Observable,
    {
        fn set_changing(&mut self, clear_cache: bool) {
            let opt = std::mem::take(&mut self.state);
            match opt {
                None => (),
                Some(obs) => match obs.set_waiting(clear_cache) {
                    ObserverState::Live(_content) => unreachable!(),
                    other => self.state = Some(other),
                },
            }
        }

        fn set_live(
            &mut self,
            content: Option<Result<O::T, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            let opt = std::mem::take(&mut self.state);
            match opt {
                None => (),
                Some(obs) => match obs.set_live(content) {
                    ObserverState::Live(content) => {
                        let tx = std::mem::take(&mut self.tx);
                        if let Some(tx) = tx {
                            let _ = tx.send(content);
                        }
                    }
                    other => self.state = Some(other),
                },
            };
            Box::new(std::future::ready(()))
        }
    }
}

mod map {
    use super::*;

    pub struct Map<O, F>
    where
        O: Observable,
        F: FnMut(O::T) -> O::T,
    {
        observable: O,
        f: F,
    }

    impl<O, F> Map<O, F>
    where
        O: Observable,
        F: FnMut(O::T) -> O::T,
    {
        pub fn new(observable: O, f: F) -> Map<O, F> {
            Map { observable, f }
        }
    }

    impl<O, F> Observable for Map<O, F>
    where
        O: Observable,
        F: FnMut(O::T) -> O::T + Send + 'static,
    {
        type T = O::T;
        type E = O::E;
        fn attach<P: Observer<O::T, O::E> + 'static>(
            self,
            observer: P,
        ) -> impl FnOnce() -> (Self, P) + Send {
            // attach
            let detach = self.observable.attach(MapObserver {
                f: self.f,
                next: observer,
                phantom: PhantomData,
            });
            // detach fn
            || {
                let (
                    observable,
                    MapObserver {
                        next,
                        f,
                        phantom: _,
                    },
                ) = detach();
                (Map { observable, f }, next)
            }
        }
    }

    pub struct MapObserver<T, E, P, F>
    where
        P: Observer<T, E>,
        F: FnMut(T) -> T,
    {
        next: P,
        f: F,
        phantom: PhantomData<(T, E)>,
    }

    impl<T, E, P, F> Observer<T, E> for MapObserver<T, E, P, F>
    where
        P: Observer<T, E>,
        F: FnMut(T) -> T + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        fn set_changing(&mut self, clear_cache: bool) {
            self.next.set_changing(clear_cache)
        }
        fn set_live(
            &mut self,
            content: Option<Result<T, E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            match content {
                Some(Ok(value)) => self.next.set_live(Some(Ok((self.f)(value)))),
                Some(Err(e)) => self.next.set_live(Some(Err(e))),
                None => self.next.set_live(None),
            }
        }
    }
}

// struct Subject<T, E> {
//     state: ObserverState<T, E>,
//     observer: Vec<Box<dyn Observer<T, E>>>,
// }
//
// impl<T, E> Observable for &Subject<T, E>
// where
//     T: Send + Copy,
//     E: Send + Copy,
// {
//     type T = T;
//     type E = E;
//     fn attach(self, observer: impl Observer<Self::T, Self::E>) -> AttachedObserver {
//         todo!();
//     }
// }
//
// impl<T, E> Observer<T, E> for &Subject<T, E> {
//     fn set_waiting(&mut self, clear_cache: bool) {}
//     fn set_live(&mut self, content: Option<Result<T, E>>) -> Box<dyn Future<Output = ()> + Sync> {
//         todo!()
//     }
// }

mod share {
    use super::*;

    pub struct Share<O>
    where
        O: Observable,
    {
        state: std::sync::Mutex<State<O>>,
    }

    enum State<O>
    where
        O: Observable,
    {
        Detached(Option<O>),
        Attached(AttachedState<O>),
    }

    struct AttachedState<O: Observable> {
        detach_fn: Box<dyn FnOnce() -> (O, ShareObserver<O>) + Send>,
        next_observer_id: u64,
        observers: BTreeMap<u64, Box<dyn Observer<O::T, O::E>>>,
        observer_state: ObserverState<O::T, O::E>,
    }

    impl<O> Share<O>
    where
        O: Observable,
    {
        pub fn new(observable: O) -> Share<O> {
            Share {
                state: std::sync::Mutex::new(State::Detached(Some(observable))),
            }
        }
    }

    impl<O> Observable for Arc<Share<O>>
    where
        O: Observable + Send,
        O::T: Clone,
        O::E: Clone,
    {
        type T = O::T;
        type E = O::E;
        fn attach<P: Observer<O::T, O::E> + 'static>(
            self,
            observer: P,
        ) -> impl FnOnce() -> (Self, P) + Send {
            let mut observer = observer;
            let mut state = self.state.lock().unwrap();
            let observer_id = match &mut *state {
                State::Detached(observable) => {
                    // attach can only happen once so unwrap is ok
                    let observable = std::mem::take(observable).unwrap();
                    let mut observers: BTreeMap<u64, Box<dyn Observer<O::T, O::E>>> =
                        BTreeMap::new();
                    observers.insert(0, Box::new(observer));
                    let detach_fn = observable.attach(ShareObserver(self.clone()));
                    *state = State::Attached(AttachedState {
                        detach_fn: Box::new(detach_fn),
                        next_observer_id: 1,
                        observers,
                        observer_state: ObserverState::Waiting(None),
                    });
                    0
                }
                State::Attached(attached) => {
                    let observer_id = attached.next_observer_id;
                    attached.next_observer_id += 1;
                    match &attached.observer_state {
                        ObserverState::Waiting(None) => todo!("do we need to propagate waiting on subscribe? (missing changing/waiting decision)"),
                        ObserverState::Waiting(Some(_cache)) => todo!("store cache for observer"),
                        ObserverState::Live(content) => {
                            let _ = observer.set_live(Some(content.clone()));
                        }
                    }
                    attached.observers.insert(observer_id, Box::new(observer));
                    observer_id
                }
            };
            // release mutex guard to reuse `self`
            drop(state);
            move || {
                let mut state = self.state.lock().unwrap();

                let observer = match &mut *state {
                    State::Detached(_observable) => unreachable!(),
                    State::Attached(attached) => {
                        let observer_box = attached.observers.remove(&observer_id).unwrap();
                        let observer_box = (observer_box as Box<dyn Any>).downcast::<P>().unwrap();
                        let observer = *observer_box;
                        if attached.observers.is_empty() {
                            if let State::Attached(attached) =
                                std::mem::replace(&mut *state, State::Detached(None))
                            {
                                let (observable, _) = (attached.detach_fn)();
                                *state = State::Detached(Some(observable));
                            } else {
                                unreachable!()
                            }
                        }
                        observer
                    }
                };
                drop(state);

                (self, observer)
            }
        }
    }

    struct ShareObserver<O>(Arc<Share<O>>)
    where
        O: Observable;

    impl<O> Observer<O::T, O::E> for ShareObserver<O>
    where
        O: Observable + Send,
    {
        fn set_changing(&mut self, _clear_cache: bool) {
            let _ = self.0;
            todo!()
        }
        fn set_live(
            &mut self,
            _content: Option<Result<O::T, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            todo!()
        }
    }
}
