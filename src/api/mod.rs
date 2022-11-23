use rocket::Route;

pub mod chapter;
pub mod login;
pub mod manga;
pub mod reading;
pub mod register;
pub mod search;

pub fn routes() -> Vec<Route> {
    let mut routes = manga::routes();
    routes.append(&mut chapter::routes());
    routes.append(&mut login::routes());
    routes.append(&mut reading::routes());

    return routes![];
}
