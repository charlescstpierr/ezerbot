// Ezerbot: copie de tiny_http 0.12.0 avec ce seul fichier corrigé (voir vendor/tiny_http/EZERBOT.md).
//
// L'original comptait les fils au repos avec un compteur que le fil réveillé ne décrémentait
// qu'en reprenant le verrou. Deux connexions arrivées avant ce réveil voyaient le même fil
// « libre » : la seconde restait dans la file sans fil pour la lire, tant qu'une connexion
// keep-alive gardait son fil (indéfiniment avec un navigateur ouvert).
// Ici, chaque tâche mise en file réserve un fil au repos sous le même verrou que la file.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

/// Manages a collection of threads.
///
/// A new thread is created every time all the existing threads are full.
/// Any idle thread will automatically die after a few seconds.
pub struct TaskPool {
    sharing: Arc<Sharing>,
}

struct Queue {
    // list of the tasks to be done by worker threads
    todo: VecDeque<Box<dyn FnMut() + Send>>,

    // idle worker threads not yet promised to a queued task:
    // threads waiting on the condvar minus tasks in `todo`
    free: usize,
}

struct Sharing {
    queue: Mutex<Queue>,

    // condvar that will be notified whenever a task is added to `todo`
    condvar: Condvar,

    // number of total worker threads running
    active_tasks: AtomicUsize,
}

/// Minimum number of active threads.
static MIN_THREADS: usize = 4;

struct Registration<'a> {
    nb: &'a AtomicUsize,
}

impl<'a> Registration<'a> {
    fn new(nb: &'a AtomicUsize) -> Registration<'a> {
        nb.fetch_add(1, Ordering::Release);
        Registration { nb }
    }
}

impl<'a> Drop for Registration<'a> {
    fn drop(&mut self) {
        self.nb.fetch_sub(1, Ordering::Release);
    }
}

impl TaskPool {
    pub fn new() -> TaskPool {
        let pool = TaskPool {
            sharing: Arc::new(Sharing {
                queue: Mutex::new(Queue {
                    todo: VecDeque::new(),
                    free: 0,
                }),
                condvar: Condvar::new(),
                active_tasks: AtomicUsize::new(0),
            }),
        };

        for _ in 0..MIN_THREADS {
            pool.add_thread(None)
        }

        pool
    }

    /// Executes a function in a thread.
    /// If no thread is available, spawns a new one.
    pub fn spawn(&self, code: Box<dyn FnMut() + Send>) {
        let mut queue = self.sharing.queue.lock().unwrap();

        if queue.free == 0 {
            drop(queue);
            self.add_thread(Some(code));
        } else {
            queue.free -= 1;
            queue.todo.push_back(code);
            self.sharing.condvar.notify_one();
        }
    }

    fn add_thread(&self, initial_fn: Option<Box<dyn FnMut() + Send>>) {
        let sharing = self.sharing.clone();

        thread::spawn(move || {
            let sharing = sharing;
            let _active_guard = Registration::new(&sharing.active_tasks);

            if let Some(mut f) = initial_fn {
                f();
            }

            loop {
                let mut task: Box<dyn FnMut() + Send> = {
                    let mut queue = sharing.queue.lock().unwrap();

                    if let Some(task) = queue.todo.pop_front() {
                        // Taken without waiting: the idle thread promised to it stays free.
                        queue.free += 1;
                        task
                    } else {
                        queue.free += 1;
                        loop {
                            let timed_out =
                                if sharing.active_tasks.load(Ordering::Acquire) <= MIN_THREADS {
                                    queue = sharing.condvar.wait(queue).unwrap();
                                    false
                                } else {
                                    let (new_lock, waitres) = sharing
                                        .condvar
                                        .wait_timeout(queue, Duration::from_millis(5000))
                                        .unwrap();
                                    queue = new_lock;
                                    waitres.timed_out()
                                };

                            // A queued task already removed this thread from `free`.
                            if let Some(task) = queue.todo.pop_front() {
                                break task;
                            }
                            if timed_out {
                                queue.free -= 1;
                                return;
                            }
                        }
                    }
                };

                task();
            }
        });
    }
}

impl Drop for TaskPool {
    fn drop(&mut self) {
        self.sharing
            .active_tasks
            .store(999_999_999, Ordering::Release);
        self.sharing.condvar.notify_all();
    }
}
