#[macro_use]
extern crate diesel;

use chrono_tz::Tz;
use diesel::{
    connection::Connection, pg::PgConnection, BoolExpressionMethods, ExpressionMethods, QueryDsl,
    RunQueryDsl,
};
use serenity::{
    client::{Client, Context},
    framework::standard::{
        help_commands,
        macros::{command, group, help},
        ArgError, Args, CommandGroup, CommandResult, Delimiter, HelpOptions, StandardFramework,
    },
    http::Http,
    model::{channel::Message, id::GuildId, id::RoleId, id::UserId},
    utils::TypeMapKey,
};
use std::{collections::HashSet, env, sync::Arc, time::SystemTime};
// Reconsider the use of this Mutex
use tokio::{sync::Mutex, time};

mod schema;

use schema::roles;

#[derive(Insertable, Queryable)]
#[table_name = "roles"]
pub struct Role<'a> {
    pub id: i64,
    pub guild_id: i64,
    pub timezone: &'a str,
}

fn role_name(tz: Tz) -> String {
    use chrono::offset::TimeZone;
    format!(
        "{} {}",
        tz.name(),
        tz.timestamp_millis(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
        )
        .format("%R")
    )
}

#[command]
async fn add(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    // TODO: check that the timezone doesn't already exist on the server
    match args.trimmed().single::<Tz>() {
        Ok(timezone) => {
            diesel::insert_into(roles::table)
                .values(Role {
                    id: *msg
                        .guild_id
                        .unwrap()
                        .create_role(&ctx.http, |r| r.name(role_name(timezone)))
                        .await
                        .unwrap()
                        .id
                        .as_u64() as i64,
                    guild_id: *msg.guild_id.unwrap().as_u64() as i64,
                    timezone: timezone.name(),
                })
                .execute(&*ctx.data.read().await.get::<DbConn>().unwrap().lock().await)
                .unwrap();
            msg.channel_id
                .say(&ctx.http, format!("Added timezone {}", timezone.name()))
                .await
                .unwrap();
        }
        Err(ArgError::Parse(error)) => {
            msg.channel_id.say(&ctx.http, error).await.unwrap();
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[command]
async fn list(ctx: &Context, msg: &Message) -> CommandResult {
    // TODO: mention the role?
    // https://support.discord.com/hc/en-us/community/posts/360039210411--Suggestion-Mentioning-without-pinging
    //
    // TODO: add DM option
    // TODO: deal with sending empty messages (when no roles)
    let roles = roles::table
        .filter(roles::guild_id.eq(*msg.guild_id.unwrap().as_u64() as i64))
        .select(roles::timezone)
        .load::<String>(&*ctx.data.read().await.get::<DbConn>().unwrap().lock().await)
        .unwrap()
        .join("\n");
    msg.channel_id.say(&ctx.http, roles).await.unwrap();
    Ok(())
}

#[command]
#[usage = "all|<timezone>"]
#[description = "remove roles"]
async fn purge(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    // TODO: use a generic function to prevent the repetition of deleting
    let roles_to_remove = match args.single::<String>() {
        Ok(arg) => match arg.as_str() {
            "all" => diesel::delete(
                roles::table.filter(roles::guild_id.eq(*msg.guild_id.unwrap().as_u64() as i64)),
            )
            .returning(roles::id)
            .get_results::<i64>(&*ctx.data.read().await.get::<DbConn>().unwrap().lock().await)
            .unwrap(),
            timezone => match timezone.parse::<Tz>() {
                Ok(timezone) => diesel::delete(
                    roles::table.filter(
                        roles::guild_id
                            .eq(*msg.guild_id.unwrap().as_u64() as i64)
                            .and(roles::timezone.eq(timezone.name())),
                    ),
                )
                .returning(roles::id)
                .get_results::<i64>(&*ctx.data.read().await.get::<DbConn>().unwrap().lock().await)
                .unwrap(),
                Err(error) => {
                    msg.channel_id.say(&ctx.http, error).await.unwrap();
                    return Ok(());
                }
            },
        },
        Err(ArgError::Eos) => {
            (MY_HELP.fun)(
                ctx,
                msg,
                Args::new("purge", &[Delimiter::Single(' ')]),
                &MY_HELP.options,
                &[&GENERAL_GROUP],
                HashSet::new(),
            )
            .await
            .unwrap();
            return Ok(());
        }
        _ => unimplemented!(),
    };
    for id in roles_to_remove.iter() {
        msg.guild_id
            .unwrap()
            .delete_role(&ctx.http, *id as u64)
            .await
            .unwrap();
    }
    // TODO: role(s) (depending on the number of roles purged)
    msg.channel_id
        .say(&ctx.http, format!("Purged {} roles", roles_to_remove.len()))
        .await
        .unwrap();
    Ok(())
}

#[group]
#[commands(add, list, purge)]
struct General;

struct DbConn;
impl TypeMapKey for DbConn {
    type Value = Arc<Mutex<PgConnection>>;
}

#[help]
async fn my_help(
    ctx: &Context,
    msg: &Message,
    args: Args,
    help_options: &'static HelpOptions,
    groups: &[&'static CommandGroup],
    owners: HashSet<UserId>,
) -> CommandResult {
    help_commands::plain(ctx, msg, args, help_options, groups, owners).await
}

#[tokio::main]
async fn main() {
    let token = env::var("DISCORD_TOKEN").unwrap();
    //TODO: An http client is created, and then unused even though serenity also creates one
    let http = Http::new_with_token(&token);
    let bot_id = http.get_current_user().await.unwrap().id;
    let mut client = Client::new(&token)
        .framework(
            StandardFramework::new()
                .configure(|c| c.on_mention(Some(bot_id)))
                .help(&MY_HELP)
                .group(&GENERAL_GROUP),
        )
        .await
        .unwrap();
    let dbconn = Arc::new(Mutex::new(
        PgConnection::establish(&env::var("DATABASE_URL").unwrap()).unwrap(),
    ));
    client
        .data
        .write()
        .await
        .insert::<DbConn>(Arc::clone(&dbconn));
    let mut interval = time::interval(time::Duration::from_secs(10));
    tokio::spawn(async move {
        loop {
            interval.tick().await;
            // TODO: use the Role struct
            let roles = roles::table
                .select((roles::id, roles::guild_id, roles::timezone))
                .load::<(i64, i64, String)>(&*dbconn.lock().await)
                .unwrap();
            for role in roles {
                GuildId(role.1 as u64)
                    .edit_role(&http, RoleId(role.0 as u64), |r| {
                        r.name(role_name(role.2.parse().unwrap()))
                    })
                    .await
                    .unwrap();
            }
        }
    });
    client.start().await.unwrap();
}
