use luna::{Callback, CallbackReturn, Context, Function, Table, Value as LuaValue};
use mold_io::Timer as IoTimer;
use mold_scene::{Element, VirtualList};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::Duration;

use crate::{configure::*, scene_bindings::*, state::*, table_menu::*, types::*, views::*};

pub(crate) fn element_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
    element: Element,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let properties: Table = stack.consume(ctx)?;
        let node = create_node(&state, element);
        configure_element(&state, ctx, limits, node, properties).map_err(HostError)?;
        if element == Element::Inset
            && state
                .borrow()
                .scene
                .children(node)
                .map_err(|error| HostError(error.to_string()))?
                .len()
                > 1
        {
            return Err(HostError("Inset accepts at most one child".into()).into());
        }
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn loader_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let properties: Table = stack.consume(ctx)?;
        let clean = Table::new(&ctx);
        let mut source = None;
        for (key, value) in properties.iter(ctx) {
            if matches!(key, LuaValue::String(name) if name.display_lossy().to_string() == "source")
            {
                let LuaValue::Function(Function::Closure(factory)) = value else {
                    return Err(HostError("Loader source must be a function".into()).into());
                };
                source = Some(ctx.stash(factory));
            } else {
                clean.set(ctx, key, value)?;
            }
        }
        let node = create_node(&state, Element::Loader);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        if let Some(source) = source.clone() {
            state.borrow_mut().loader_factories.insert(node, source);
        }
        if state
            .borrow()
            .scene
            .bool_value(node, "active")
            .map_err(|error| HostError(error.to_string()))?
            && let Some(source) = source
        {
            let child = execute_node_factory(ctx, &source, limits).map_err(HostError)?;
            state
                .borrow_mut()
                .scene
                .reparent(child, Some(node))
                .map_err(|error| HostError(error.to_string()))?;
            state.borrow_mut().loaded_loaders.insert(node);
        }
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn timer_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let properties: Table = stack.consume(ctx)?;
        let clean = Table::new(&ctx);
        let mut callback = None;
        for (key, value) in properties.iter(ctx) {
            if matches!(key, LuaValue::String(name) if name.display_lossy().to_string() == "on_triggered")
            {
                let LuaValue::Function(Function::Closure(closure)) = value else {
                    return Err(HostError("Timer on_triggered must be a function".into()).into());
                };
                callback = Some(ctx.stash(closure));
            } else {
                clean.set(ctx, key, value)?;
            }
        }
        let node = create_node(&state, Element::Timer);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        let (interval, repeat, running) = {
            let state = state.borrow();
            let interval = state
                .scene
                .number(node, "interval")
                .map_err(|error| HostError(error.to_string()))?;
            let repeat = state
                .scene
                .bool_value(node, "repeat")
                .map_err(|error| HostError(error.to_string()))?;
            let running = state
                .scene
                .bool_value(node, "running")
                .map_err(|error| HostError(error.to_string()))?;
            (interval, repeat, running)
        };
        if running {
            if !interval.is_finite() || interval <= 0.0 {
                return Err(HostError("Timer interval must be finite and positive".into()).into());
            }
            let callback =
                callback.ok_or_else(|| HostError("running Timer requires on_triggered".into()))?;
            let timer = IoTimer::every(Duration::from_secs_f64(interval / 1_000.0))
                .map_err(|error| HostError(error.to_string()))?;
            let interval = Duration::from_secs_f64(interval / 1_000.0);
            state.borrow_mut().timers.push(PendingTimer {
                timer,
                callback: callback.clone(),
                repeat,
                interval,
                node: Some(node),
            });
            state.borrow_mut().timer_callbacks.insert(node, callback);
        } else if let Some(callback) = callback {
            state.borrow_mut().timer_callbacks.insert(node, callback);
        }
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn view_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
    kind: ViewKind,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let virtualized = !matches!(kind, ViewKind::Repeater);
        let properties: Table = stack.consume(ctx)?;
        let model = match properties.get_value(ctx, "model") {
            LuaValue::UserData(model) => model
                .downcast_static::<ListModelToken>()
                .map_err(|_| HostError("view model must be a mold list model".to_owned()))?,
            _ => return Err(HostError("view model must be a mold list model".to_owned()).into()),
        };
        let delegate = match properties.get_value(ctx, "delegate") {
            LuaValue::Function(Function::Closure(delegate)) => ctx.stash(delegate),
            _ => return Err(HostError("view delegate must be a function".to_owned()).into()),
        };
        let clean = Table::new(&ctx);
        for (key, value) in properties.iter(ctx) {
            let special = matches!(
                key,
                LuaValue::String(name)
                    if matches!(
                        name.display_lossy().to_string().as_str(),
                        "model"
                            | "delegate"
                            | "item_extent"
                            | "overscan"
                            | "content_y"
                            | "cell_width"
                            | "cell_height"
                            | "columns"
                    )
            );
            if !special {
                clean
                    .set(ctx, key, value)
                    .map_err(|error| HostError(error.to_string()))?;
            }
        }
        if virtualized {
            clean.set_field(ctx, "clip", true);
        }
        let node = create_node(&state, Element::Item);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        let model_handle = Rc::clone(&model.model);
        let model = model_handle.borrow();
        let mut configured_view = None;
        let (range, item_extent, offset, columns, column_extent) = match kind {
            ViewKind::Repeater => (0..model.len(), 0.0, 0.0, 1, 0.0),
            ViewKind::List => {
                let item_extent =
                    table_number(ctx, properties, "item_extent", 1.0).map_err(HostError)?;
                let height = table_number(ctx, properties, "height", 0.0).map_err(HostError)?;
                let offset = table_number(ctx, properties, "content_y", 0.0).map_err(HostError)?;
                let overscan = table_number(ctx, properties, "overscan", 1.0).map_err(HostError)?;
                if item_extent <= 0.0 || height < 0.0 || offset < 0.0 || overscan < 0.0 {
                    return Err(HostError("invalid ListView dimensions".to_owned()).into());
                }
                let mut view = VirtualList::new(item_extent, height, overscan as usize)
                    .ok_or_else(|| HostError("invalid ListView dimensions".to_owned()))?;
                view.set_offset(offset);
                let range = view.visible_range(model.len());
                configured_view = Some(view);
                (range, item_extent, offset, 1, 0.0)
            }
            ViewKind::Grid => {
                let cell_width =
                    table_number(ctx, properties, "cell_width", 1.0).map_err(HostError)?;
                let cell_height =
                    table_number(ctx, properties, "cell_height", 1.0).map_err(HostError)?;
                let width = table_number(ctx, properties, "width", 0.0).map_err(HostError)?;
                let height = table_number(ctx, properties, "height", 0.0).map_err(HostError)?;
                let offset = table_number(ctx, properties, "content_y", 0.0).map_err(HostError)?;
                let overscan = table_number(ctx, properties, "overscan", 1.0).map_err(HostError)?;
                let default_columns = (width / cell_width).floor().max(1.0);
                let columns =
                    table_number(ctx, properties, "columns", default_columns).map_err(HostError)?;
                if cell_width <= 0.0
                    || cell_height <= 0.0
                    || width < 0.0
                    || height < 0.0
                    || offset < 0.0
                    || overscan < 0.0
                    || columns < 1.0
                    || columns.fract() != 0.0
                {
                    return Err(HostError("invalid GridView dimensions".to_owned()).into());
                }
                let columns = columns as usize;
                let mut view =
                    VirtualList::new_grid(cell_height, height, overscan as usize, columns)
                        .ok_or_else(|| HostError("invalid GridView dimensions".to_owned()))?;
                view.set_offset(offset);
                let range = view.visible_range(model.len());
                configured_view = Some(view);
                (range, cell_height, offset, columns, cell_width)
            }
        };
        let reuse_limit = range.len().max(1) * 2;
        let mut active = HashMap::new();
        for index in range {
            let (id, item) = model
                .get(index)
                .expect("view range contains live model indexes");
            let child = execute_delegate(ctx, &delegate, item, index, limits).map_err(HostError)?;
            if virtualized {
                position_view_child(
                    &mut state.borrow_mut().scene,
                    child.node,
                    index,
                    item_extent,
                    offset,
                    columns,
                    column_extent,
                )
                .map_err(HostError)?;
            }
            state
                .borrow_mut()
                .scene
                .reparent(child.node, Some(node))
                .map_err(|error| HostError(error.to_string()))?;
            active.insert(id, child);
        }
        drop(model);
        if let Some(mut view) = configured_view {
            let _ = view.sync(&model_handle.borrow(), &[]);
            state.borrow_mut().views.insert(
                node,
                LuaVirtualView {
                    model: model_handle,
                    view,
                    delegate,
                    active,
                    reusable: HashMap::new(),
                    reuse_order: VecDeque::new(),
                    reuse_limit,
                    pool_root: None,
                    column_extent,
                },
            );
        }
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}
